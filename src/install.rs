use crate::domain::{HookDoctorReport, HookTargetDoctorReport, InstallReport};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const SKILL: &str = include_str!("../skills/agent-sync-resume/SKILL.md");
const MARKER: &str = "# agent-sync-managed-hook";

#[derive(Debug, Copy, Clone)]
pub enum InstallTarget {
    Codex,
    Claude,
    All,
}

pub fn install(target: InstallTarget) -> Result<Vec<InstallReport>> {
    let mut reports = Vec::new();
    match target {
        InstallTarget::Codex => reports.push(install_codex()?),
        InstallTarget::Claude => reports.push(install_claude()?),
        InstallTarget::All => {
            reports.push(install_codex()?);
            reports.push(install_claude()?);
        }
    }
    Ok(reports)
}

fn install_codex() -> Result<InstallReport> {
    let home = dirs::home_dir().context("home directory not found")?;
    let mut backups = Vec::new();
    let skill_dir = home.join(".codex/skills/agent-sync-resume");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL)?;

    let hooks_path = home.join(".codex/hooks.json");
    let mut root = read_json_or_object(&hooks_path)?;
    ensure_hook(
        &mut root,
        "SessionStart",
        Some("startup|resume"),
        "agent-sync hook codex",
    );
    ensure_hook(&mut root, "UserPromptSubmit", None, "agent-sync hook codex");
    ensure_hook(&mut root, "Stop", None, "agent-sync hook codex");
    backup_if_exists(&hooks_path, &mut backups)?;
    write_json(&hooks_path, &root)?;

    Ok(InstallReport {
        target: "codex".to_string(),
        skill_installed: true,
        hooks_installed: true,
        backups,
        warnings: Vec::new(),
        doctor: Some(validate_target("codex")?),
    })
}

fn install_claude() -> Result<InstallReport> {
    let home = dirs::home_dir().context("home directory not found")?;
    let mut backups = Vec::new();
    let skill_dir = home.join(".claude/skills/agent-sync-resume");
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL)?;

    let settings_path = home.join(".claude/settings.json");
    let mut root = read_json_or_object(&settings_path)?;
    ensure_hook(&mut root, "SessionStart", None, "agent-sync hook claude");
    ensure_hook(
        &mut root,
        "UserPromptSubmit",
        None,
        "agent-sync hook claude",
    );
    ensure_hook(&mut root, "Stop", None, "agent-sync hook claude");
    ensure_hook(&mut root, "SessionEnd", None, "agent-sync hook claude");
    backup_if_exists(&settings_path, &mut backups)?;
    write_json(&settings_path, &root)?;

    Ok(InstallReport {
        target: "claude".to_string(),
        skill_installed: true,
        hooks_installed: true,
        backups,
        warnings: Vec::new(),
        doctor: Some(validate_target("claude")?),
    })
}

pub fn doctor() -> Result<HookDoctorReport> {
    let targets = vec![validate_target("codex")?, validate_target("claude")?];
    let ok = targets.iter().all(|target| {
        target.skill_installed && target.config_valid_json && target.managed_hooks_present
    });
    Ok(HookDoctorReport { targets, ok })
}

fn validate_target(target: &str) -> Result<HookTargetDoctorReport> {
    let home = dirs::home_dir().context("home directory not found")?;
    let (skill_path, config_path) = match target {
        "codex" => (
            home.join(".codex/skills/agent-sync-resume/SKILL.md"),
            home.join(".codex/hooks.json"),
        ),
        "claude" => (
            home.join(".claude/skills/agent-sync-resume/SKILL.md"),
            home.join(".claude/settings.json"),
        ),
        _ => anyhow::bail!("unknown install target: {target}"),
    };
    let mut warnings = Vec::new();
    let skill_installed = skill_path.exists();
    if !skill_installed {
        warnings.push(format!("skill missing at {}", skill_path.display()));
    }
    let config_exists = config_path.exists();
    let mut config_valid_json = false;
    let mut managed_hooks_present = false;
    if config_exists {
        match fs::read(&config_path)
            .with_context(|| format!("read {}", config_path.display()))
            .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
        {
            Ok(value) => {
                config_valid_json = true;
                managed_hooks_present = contains_managed_hook(&value);
                if !managed_hooks_present {
                    warnings.push(format!("managed hook missing in {}", config_path.display()));
                }
            }
            Err(err) => warnings.push(format!("invalid hook config: {err:#}")),
        }
    } else {
        warnings.push(format!("config missing at {}", config_path.display()));
    }
    Ok(HookTargetDoctorReport {
        target: target.to_string(),
        skill_path,
        config_path,
        skill_installed,
        config_exists,
        config_valid_json,
        managed_hooks_present,
        warnings,
    })
}

fn read_json_or_object(path: &Path) -> Result<Value> {
    if path.exists() {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(json!({}))
    }
}

fn ensure_hook(root: &mut Value, event: &str, matcher: Option<&str>, command: &str) {
    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let event_hooks = hooks
        .as_object_mut()
        .unwrap()
        .entry(event)
        .or_insert_with(|| json!([]));
    let arr = event_hooks.as_array_mut().unwrap();
    let command = format!("{command} {MARKER}");
    let exists = arr.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(|hooks| hooks.as_array())
            .map(|hooks| {
                hooks.iter().any(|hook| {
                    hook.get("command")
                        .and_then(|command| command.as_str())
                        .map(|command| command.contains(MARKER))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });
    if exists {
        return;
    }
    let mut entry = json!({
        "hooks": [{
            "type": "command",
            "command": command
        }]
    });
    if let Some(matcher) = matcher {
        entry["matcher"] = json!(matcher);
    }
    arr.push(entry);
}

fn contains_managed_hook(value: &Value) -> bool {
    match value {
        Value::String(value) => value.contains(MARKER),
        Value::Array(values) => values.iter().any(contains_managed_hook),
        Value::Object(values) => values.values().any(contains_managed_hook),
        _ => false,
    }
}

fn backup_if_exists(path: &Path, backups: &mut Vec<PathBuf>) -> Result<()> {
    if path.exists() {
        let backup = path.with_extension(format!("{}.bak", Utc::now().format("%Y%m%d%H%M%S")));
        fs::copy(path, &backup)?;
        backups.push(backup);
    }
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
