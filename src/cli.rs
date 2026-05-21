use crate::app::AgentSync;
use crate::config::Config;
use crate::{daemon, hooks, install, mcp};
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agent-sync",
    version,
    about = "Agent coding session continuity service"
)]
struct Args {
    #[arg(long, env = "AGENT_SYNC_ROOT")]
    sync_root: Option<PathBuf>,
    #[arg(long, env = "AGENT_SYNC_CACHE")]
    cache_root: Option<PathBuf>,
    #[arg(long, env = "AGENT_SYNC_HOSTNAME")]
    hostname: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Recent {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    List {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    Handoff {
        conversation_id: String,
    },
    Claim {
        conversation_id: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    Resume {
        conversation_id: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    Refresh {
        conversation_id: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    Sandbox {
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    Checkpoint {
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(long)]
        new: bool,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        summary: Option<String>,
        #[arg(long)]
        last_assistant_message: Option<String>,
    },
    ApplyDirty {
        checkpoint_id: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Install {
        target: InstallTargetArg,
    },
    ValidateSync,
    Storage,
    Prune {
        #[arg(long)]
        execute: bool,
        #[arg(long)]
        older_than: Option<String>,
    },
    Mcp,
    Daemon,
    Hook {
        format: HookFormat,
    },
    Status,
    Doctor {
        #[arg(long)]
        hooks: bool,
        #[arg(long)]
        storage: bool,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum HookFormat {
    Claude,
    Codex,
}

#[derive(Copy, Clone, ValueEnum)]
enum InstallTargetArg {
    Codex,
    Claude,
    All,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let config = Config::resolve(args.sync_root, args.cache_root, args.hostname)?;
    match args.command {
        Command::Recent { limit } | Command::List { limit } => {
            let app = AgentSync::new(config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&app.list_recent_summaries(limit)?)?
            );
            Ok(())
        }
        Command::Handoff { conversation_id } => {
            let app = AgentSync::new(config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&app.get_handoff_plan(&conversation_id)?)?
            );
            Ok(())
        }
        Command::Claim {
            conversation_id,
            cwd,
        } => {
            let app = AgentSync::new(config)?;
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            println!(
                "{}",
                serde_json::to_string_pretty(&app.claim_conversation(&conversation_id, cwd)?)?
            );
            Ok(())
        }
        Command::Resume {
            conversation_id,
            cwd,
        } => {
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            let sandbox = crate::app::detect_sandbox_for_config(&config, Some(cwd.clone()));
            if !sandbox.sync_root_writable || sandbox.git_metadata_writable == Some(false) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "conversation_id": conversation_id,
                        "resumed": false,
                        "sandbox": sandbox,
                        "commands": [],
                    }))?
                );
                return Ok(());
            }
            let app = AgentSync::new(config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&app.resume_conversation(&conversation_id, cwd)?)?
            );
            Ok(())
        }
        Command::Refresh {
            conversation_id,
            cwd,
        } => {
            let app = AgentSync::new(config)?;
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &app.refresh_conversation_repo(&conversation_id, cwd)?
                )?
            );
            Ok(())
        }
        Command::Sandbox { cwd } => {
            let cwd = match cwd {
                Some(cwd) => Some(cwd),
                None => std::env::current_dir().ok(),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::app::detect_sandbox_for_config(&config, cwd))?
            );
            Ok(())
        }
        Command::Checkpoint {
            cwd,
            conversation_id,
            new,
            title,
            summary,
            last_assistant_message,
        } => {
            if new && conversation_id.is_some() {
                anyhow::bail!("--new cannot be combined with --conversation-id");
            }
            let app = AgentSync::new(config)?;
            let checkpoint = app.create_checkpoint(crate::app::CheckpointInput {
                cwd: cwd.unwrap_or(std::env::current_dir()?),
                title,
                conversation_id,
                new_conversation: new,
                summary,
                last_assistant_message,
                provenance: crate::domain::HookProvenance::default(),
            })?;
            println!("{}", serde_json::to_string_pretty(&checkpoint)?);
            Ok(())
        }
        Command::ApplyDirty {
            checkpoint_id,
            cwd,
            force,
        } => {
            let app = AgentSync::new(config)?;
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            println!(
                "{}",
                serde_json::to_string_pretty(&app.apply_dirty(&checkpoint_id, cwd, force)?)?
            );
            Ok(())
        }
        Command::Install { target } => {
            let target = match target {
                InstallTargetArg::Codex => install::InstallTarget::Codex,
                InstallTargetArg::Claude => install::InstallTarget::Claude,
                InstallTargetArg::All => install::InstallTarget::All,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&install::install(target)?)?
            );
            Ok(())
        }
        Command::ValidateSync => {
            let app = AgentSync::new(config)?;
            println!("{}", serde_json::to_string_pretty(&app.validate_sync()?)?);
            Ok(())
        }
        Command::Storage => {
            let app = AgentSync::new(config)?;
            println!("{}", serde_json::to_string_pretty(&app.storage_stats()?)?);
            Ok(())
        }
        Command::Prune {
            execute,
            older_than,
        } => {
            let app = AgentSync::new(config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&app.prune(execute, parse_duration(older_than)?)?)?
            );
            Ok(())
        }
        Command::Mcp => mcp::serve_stdio(config).await,
        Command::Daemon => daemon::serve(config).await,
        Command::Hook { format } => hooks::run_hook(config, format).await,
        Command::Status => {
            let app = AgentSync::new(config)?;
            println!("{}", serde_json::to_string_pretty(&app.status()?)?);
            Ok(())
        }
        Command::Doctor { hooks, storage } => {
            let app = AgentSync::new(config)?;
            if hooks || storage {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": app.status()?,
                        "storage": if storage { Some(app.storage_stats()?) } else { None },
                        "hooks": if hooks { Some(install::doctor()?) } else { None },
                    }))?
                );
            } else {
                println!("{}", app.doctor()?);
            }
            Ok(())
        }
    }
}

fn parse_duration(input: Option<String>) -> Result<Option<std::time::Duration>> {
    let Some(input) = input else {
        return Ok(None);
    };
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--older-than cannot be empty");
    }
    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split_at);
    let amount: u64 = number
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration: {trimmed}"))?;
    let seconds = match unit {
        "" | "s" => amount,
        "m" => amount * 60,
        "h" => amount * 60 * 60,
        "d" => amount * 24 * 60 * 60,
        _ => anyhow::bail!("unsupported duration unit in {trimmed}; use s, m, h, or d"),
    };
    Ok(Some(std::time::Duration::from_secs(seconds)))
}
