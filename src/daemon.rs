use crate::app::{AgentSync, CheckpointInput};
use crate::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn serve(config: Config) -> Result<()> {
    let _ = std::fs::remove_file(&config.socket_path);
    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&config.socket_path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let config = config.clone();
        tokio::spawn(async move {
            let _ = handle_client(config, stream).await;
        });
    }
}

pub async fn call(config: &Config, request: DaemonRequest) -> Result<Option<Value>> {
    let mut stream = UnixStream::connect(&config.socket_path).await?;
    let line = serde_json::to_string(&request)? + "\n";
    stream.write_all(line.as_bytes()).await?;
    let mut reader = BufReader::new(stream);
    let mut out = String::new();
    reader.read_line(&mut out).await?;
    let response: DaemonResponse = serde_json::from_str(&out)?;
    if response.ok {
        Ok(response.result)
    } else {
        anyhow::bail!(response.error.unwrap_or_else(|| "daemon error".to_string()))
    }
}

async fn handle_client(config: Config, stream: UnixStream) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let request: DaemonRequest = serde_json::from_str(&line)?;
    let response = handle_request(config, request).await;
    let response = match response {
        Ok(result) => DaemonResponse {
            ok: true,
            result,
            error: None,
        },
        Err(err) => DaemonResponse {
            ok: false,
            result: None,
            error: Some(format!("{err:#}")),
        },
    };
    let mut stream = reader.into_inner();
    stream
        .write_all((serde_json::to_string(&response)? + "\n").as_bytes())
        .await?;
    Ok(())
}

pub async fn handle_request(config: Config, request: DaemonRequest) -> Result<Option<Value>> {
    let app = AgentSync::new(config)?;
    let value = match request.method.as_str() {
        "list_recent_conversations" => {
            let limit = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            serde_json::to_value(app.list_recent_summaries(limit)?)?
        }
        "get_conversation" => {
            let id = required_str(&request.params, "conversation_id")?;
            serde_json::to_value(app.get_conversation(id)?)?
        }
        "get_handoff_plan" => {
            let id = required_str(&request.params, "conversation_id")?;
            serde_json::to_value(app.get_handoff_plan(id)?)?
        }
        "claim_conversation" => {
            let id = required_str(&request.params, "conversation_id")?;
            let cwd = required_str(&request.params, "cwd")?;
            serde_json::to_value(app.claim_conversation(id, cwd.into())?)?
        }
        "resume_conversation" => {
            let id = required_str(&request.params, "conversation_id")?;
            let cwd = required_str(&request.params, "cwd")?;
            serde_json::to_value(app.resume_conversation(id, cwd.into())?)?
        }
        "refresh_conversation" => {
            let id = required_str(&request.params, "conversation_id")?;
            let cwd = required_str(&request.params, "cwd")?;
            serde_json::to_value(app.refresh_conversation_repo(id, cwd.into())?)?
        }
        "detect_sandbox" => {
            let cwd = request
                .params
                .get("cwd")
                .and_then(|value| value.as_str())
                .map(Into::into);
            serde_json::to_value(app.detect_sandbox(cwd)?)?
        }
        "create_checkpoint" | "record_stop_work" => {
            let input: CheckpointInput = serde_json::from_value(request.params)?;
            serde_json::to_value(app.create_checkpoint(input)?)?
        }
        "get_status" => serde_json::to_value(app.status()?)?,
        method => anyhow::bail!("unknown daemon method {method}"),
    };
    Ok(Some(value))
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing string param {key}"))
}
