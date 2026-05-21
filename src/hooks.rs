use crate::app::{AgentSync, CheckpointInput};
use crate::cli::HookFormat;
use crate::config::Config;
use crate::domain::HookProvenance;
use anyhow::Result;
use serde::Deserialize;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct HookPayload {
    session_id: Option<String>,
    transcript_path: Option<PathBuf>,
    cwd: PathBuf,
    hook_event_name: String,
    model: Option<String>,
    last_assistant_message: Option<String>,
}

pub async fn run_hook(config: Config, format: HookFormat) -> Result<()> {
    if let Err(err) = run_hook_inner(config, format).await {
        eprintln!("agent-sync hook failed open: {err:#}");
    }
    Ok(())
}

async fn run_hook_inner(config: Config, format: HookFormat) -> Result<()> {
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin)?;
    let payload: HookPayload = serde_json::from_str(&stdin)?;
    if !matches!(payload.hook_event_name.as_str(), "Stop" | "PostCompact") {
        return Ok(());
    }
    let app = AgentSync::new(config)?;
    let summary = match payload.hook_event_name.as_str() {
        "PostCompact" => Some("Conversation compacted.".to_string()),
        "Stop" => payload.last_assistant_message.clone(),
        _ => None,
    };
    if payload.hook_event_name == "Stop"
        && summary
            .as_deref()
            .map(|summary| summary.to_lowercase().contains("checkpoint"))
            .unwrap_or(false)
    {
        return Ok(());
    }
    app.create_checkpoint(CheckpointInput {
        cwd: payload.cwd,
        title: None,
        conversation_id: None,
        new_conversation: false,
        summary,
        last_assistant_message: payload.last_assistant_message,
        provenance: HookProvenance {
            hook_format: Some(
                match format {
                    HookFormat::Claude => "claude",
                    HookFormat::Codex => "codex",
                }
                .to_string(),
            ),
            hook_event_name: Some(payload.hook_event_name),
            source_session_id: payload.session_id,
            transcript_path: payload.transcript_path,
            model: payload.model,
        },
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_hook_payload() {
        let payload: HookPayload = serde_json::from_str(
            r#"{"session_id":"s","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"Stop","model":"m","last_assistant_message":"done"}"#,
        )
        .unwrap();
        assert_eq!(payload.session_id.as_deref(), Some("s"));
        assert_eq!(payload.hook_event_name, "Stop");
    }
}
