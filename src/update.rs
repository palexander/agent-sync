use crate::config::Config;
use crate::domain::{CommandOutcome, UpdateReport, VersionCheckReport};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

const DEFAULT_REPO: &str = "palexander/agent-sync";
const VERSION_CHECK_TTL_SECONDS: i64 = 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionCache {
    latest_version: Option<String>,
    checked_at: DateTime<Utc>,
    warnings: Vec<String>,
}

pub fn update() -> Result<UpdateReport> {
    let from_version = env!("CARGO_PKG_VERSION").to_string();
    let target = release_target()?;
    let repo = std::env::var("AGENT_SYNC_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
    let archive = format!("agent-sync-{target}.tar.gz");
    let download_url = format!("https://github.com/{repo}/releases/latest/download/{archive}");
    let checksum_url = format!("{download_url}.sha256");
    let current_exe = std::env::current_exe().context("resolve current executable")?;
    let binary_path = install_path(&current_exe)?;
    let temp = std::env::temp_dir().join(format!("agent-sync-update-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp)?;

    let mut commands = Vec::new();
    commands.push(run(
        "curl",
        &["-fsSL", &download_url, "-o"],
        Some(&temp.join(&archive)),
        None,
    )?);
    commands.push(run(
        "curl",
        &["-fsSL", &checksum_url, "-o"],
        Some(&temp.join(format!("{archive}.sha256"))),
        None,
    )?);
    commands.push(run(
        "shasum",
        &["-a", "256", "-c", &format!("{archive}.sha256")],
        None,
        Some(&temp),
    )?);
    commands.push(run("tar", &["-xzf", &archive], None, Some(&temp))?);

    if let Some(failed_command) = commands
        .iter()
        .find(|command| !command.success)
        .map(|command| command.command.clone())
    {
        return Ok(UpdateReport {
            updated: false,
            from_version,
            to_version: "latest".to_string(),
            target,
            binary_path,
            commands,
            install_report: None,
            doctor: None,
            warnings: vec![format!(
                "update stopped after failed command: {failed_command}"
            )],
        });
    }

    install_binary(&temp.join("agent-sync"), &binary_path)?;

    let install_output = Command::new(&binary_path)
        .args(["install", "all"])
        .output()?;
    commands.push(command_outcome(
        &binary_path,
        &["install", "all"],
        install_output,
    ));

    let doctor_output = Command::new(&binary_path)
        .args(["doctor", "--hooks", "--storage"])
        .output()?;
    commands.push(command_outcome(
        &binary_path,
        &["doctor", "--hooks", "--storage"],
        doctor_output,
    ));

    let install_report = commands
        .iter()
        .rev()
        .find(|command| command.command.ends_with(" install all"))
        .and_then(|command| serde_json::from_str(&command.stdout).ok());
    let doctor = commands
        .iter()
        .rev()
        .find(|command| command.command.ends_with(" doctor --hooks --storage"))
        .and_then(|command| serde_json::from_str(&command.stdout).ok());
    let updated = commands.iter().all(|command| command.success);

    let mut warnings = Vec::new();
    if !updated {
        warnings.push("one or more post-install commands failed".to_string());
    }

    let _ = std::fs::remove_dir_all(&temp);

    Ok(UpdateReport {
        updated,
        from_version,
        to_version: "latest".to_string(),
        target,
        binary_path,
        commands,
        install_report,
        doctor,
        warnings,
    })
}

pub fn version_check(config: &Config) -> Result<VersionCheckReport> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let cache_path = config.cache_root.join("latest-version.json");
    let now = Utc::now();
    let cached = read_fresh_cache(&cache_path, now);
    let cache_hit = cached.is_some();
    let cache = match cached {
        Some(cache) => cache,
        None => {
            let cache = fetch_latest_version(now);
            write_cache(&cache_path, &cache)?;
            cache
        }
    };
    let update_available = cache
        .latest_version
        .as_deref()
        .map(|latest| version_is_newer(&current_version, latest))
        .unwrap_or(false);
    let instructions = if update_available {
        vec![
            "Run `agent-sync update`.".to_string(),
            "Then run `agent-sync doctor --hooks --storage`.".to_string(),
            "Then retry your original agent-sync command.".to_string(),
        ]
    } else {
        Vec::new()
    };
    Ok(VersionCheckReport {
        current_version,
        latest_version: cache.latest_version,
        update_available,
        checked_at: cache.checked_at,
        cache_hit,
        instructions,
        warnings: cache.warnings,
    })
}

fn read_fresh_cache(path: &Path, now: DateTime<Utc>) -> Option<VersionCache> {
    let cache: VersionCache = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if now.signed_duration_since(cache.checked_at) < Duration::seconds(VERSION_CHECK_TTL_SECONDS) {
        Some(cache)
    } else {
        None
    }
}

fn fetch_latest_version(checked_at: DateTime<Utc>) -> VersionCache {
    let repo = std::env::var("AGENT_SYNC_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let output = Command::new("curl")
        .args(["-fsSL", "-H", "User-Agent: agent-sync", &url])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let latest_version = serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| {
                    value
                        .get("tag_name")
                        .and_then(|tag| tag.as_str())
                        .map(|version| strip_v(version).to_string())
                });
            VersionCache {
                latest_version,
                checked_at,
                warnings: Vec::new(),
            }
        }
        Ok(output) => VersionCache {
            latest_version: None,
            checked_at,
            warnings: vec![format!(
                "latest release check failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )],
        },
        Err(err) => VersionCache {
            latest_version: None,
            checked_at,
            warnings: vec![format!("latest release check failed: {err}")],
        },
    }
}

fn write_cache(path: &Path, cache: &VersionCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(cache)?)?;
    Ok(())
}

fn strip_v(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}

fn version_is_newer(current: &str, latest: &str) -> bool {
    let current = parse_version(current);
    let latest = parse_version(latest);
    latest > current
}

fn parse_version(version: &str) -> Vec<u64> {
    strip_v(version)
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn release_target() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        os => anyhow::bail!("unsupported OS: {os}"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        arch => anyhow::bail!("unsupported architecture: {arch}"),
    };
    Ok(format!("{arch}-{os}"))
}

fn install_path(current_exe: &Path) -> Result<PathBuf> {
    if let Ok(install_dir) = std::env::var("AGENT_SYNC_INSTALL_DIR") {
        return Ok(PathBuf::from(install_dir).join("agent-sync"));
    }
    Ok(current_exe.to_path_buf())
}

fn install_binary(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination.parent().with_context(|| {
        format!(
            "install destination has no parent: {}",
            destination.display()
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let temp_destination = destination.with_extension(format!("update-{}", Uuid::new_v4()));
    std::fs::copy(source, &temp_destination)?;
    make_executable(&temp_destination)?;
    std::fs::rename(temp_destination, destination)?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn run(
    program: &str,
    args: &[&str],
    output_path: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<CommandOutcome> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(output_path) = output_path {
        command.arg(output_path);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output()?;
    Ok(command_outcome(Path::new(program), args, output))
}

fn command_outcome(program: &Path, args: &[&str], output: std::process::Output) -> CommandOutcome {
    let command = std::iter::once(program.display().to_string())
        .chain(args.iter().map(|arg| (*arg).to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    CommandOutcome {
        command,
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}
