use crate::app::{AgentSync, CheckpointInput};
use crate::config::Config;
use crate::daemon::{self, DaemonRequest};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

pub async fn serve_stdio(config: Config) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: Result<RpcRequest, _> = serde_json::from_str(&line);
        let response = match parsed {
            Ok(request) => handle_rpc(config.clone(), request).await,
            Err(err) => RpcResponse {
                jsonrpc: "2.0",
                id: None,
                result: None,
                error: Some(json!({"code": -32700, "message": err.to_string()})),
            },
        };
        stdout
            .write_all((serde_json::to_string(&response)? + "\n").as_bytes())
            .await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn handle_rpc(config: Config, request: RpcRequest) -> RpcResponse {
    let id = request.id.clone();
    if request.jsonrpc.as_deref() != Some("2.0") && id.is_some() {
        return error(id, -32600, "invalid jsonrpc version");
    }
    let result = match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "agent-sync", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}}
        })),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call_tool(config, request.params).await,
        "notifications/initialized" => {
            return RpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: None,
            }
        }
        _ => Err(anyhow::anyhow!("unknown method {}", request.method)),
    };
    match result {
        Ok(value) => RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(value),
            error: None,
        },
        Err(err) => error(id, -32603, &err.to_string()),
    }
}

async fn call_tool(config: Config, params: Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = dispatch(config, name, arguments).await?;
    Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&result)?}]}))
}

async fn dispatch(config: Config, name: &str, arguments: Value) -> Result<Value> {
    let request = DaemonRequest {
        method: name.to_string(),
        params: arguments.clone(),
    };
    match daemon::call(&config, request).await {
        Ok(Some(value)) => return Ok(value),
        Ok(None) => return Ok(json!(null)),
        Err(_) => {}
    }
    let app = AgentSync::new(config)?;
    match name {
        "list_recent_conversations" => {
            let limit = arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            Ok(serde_json::to_value(app.list_recent_summaries(limit)?)?)
        }
        "get_conversation" => {
            let id = required_str(&arguments, "conversation_id")?;
            Ok(serde_json::to_value(app.get_conversation(id)?)?)
        }
        "get_handoff_plan" => {
            let id = required_str(&arguments, "conversation_id")?;
            Ok(serde_json::to_value(app.get_handoff_plan(id)?)?)
        }
        "claim_conversation" => {
            let id = required_str(&arguments, "conversation_id")?;
            let cwd = required_str(&arguments, "cwd")?;
            Ok(serde_json::to_value(
                app.claim_conversation(id, cwd.into())?,
            )?)
        }
        "resume_conversation" => {
            let id = required_str(&arguments, "conversation_id")?;
            let cwd = required_str(&arguments, "cwd")?;
            Ok(serde_json::to_value(
                app.resume_conversation(id, cwd.into())?,
            )?)
        }
        "refresh_conversation" => {
            let id = required_str(&arguments, "conversation_id")?;
            let cwd = required_str(&arguments, "cwd")?;
            Ok(serde_json::to_value(
                app.refresh_conversation_repo(id, cwd.into())?,
            )?)
        }
        "detect_sandbox" => {
            let cwd = arguments
                .get("cwd")
                .and_then(|value| value.as_str())
                .map(Into::into);
            Ok(serde_json::to_value(app.detect_sandbox(cwd)?)?)
        }
        "create_checkpoint" | "record_stop_work" => {
            let input: CheckpointInput = serde_json::from_value(arguments)?;
            Ok(serde_json::to_value(app.create_checkpoint(input)?)?)
        }
        "get_status" => Ok(serde_json::to_value(app.status()?)?),
        _ => anyhow::bail!("unknown tool {name}"),
    }
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "list_recent_conversations",
            "List recent app-agnostic conversations",
            json!({"type":"object","properties":{"limit":{"type":"integer"}}}),
        ),
        tool(
            "get_conversation",
            "Get a conversation by id",
            json!({"type":"object","required":["conversation_id"],"properties":{"conversation_id":{"type":"string"}}}),
        ),
        tool(
            "get_handoff_plan",
            "Build a resume handoff plan",
            json!({"type":"object","required":["conversation_id"],"properties":{"conversation_id":{"type":"string"}}}),
        ),
        tool(
            "claim_conversation",
            "Claim a conversation for this machine after user confirmation",
            json!({"type":"object","required":["conversation_id","cwd"],"properties":{"conversation_id":{"type":"string"},"cwd":{"type":"string"}}}),
        ),
        tool(
            "resume_conversation",
            "Claim a conversation, fetch/pull its branch, and refresh agent-sync repo state",
            json!({"type":"object","required":["conversation_id","cwd"],"properties":{"conversation_id":{"type":"string"},"cwd":{"type":"string"}}}),
        ),
        tool(
            "refresh_conversation",
            "Refresh stored repo state for a conversation after handoff work completes",
            json!({"type":"object","required":["conversation_id","cwd"],"properties":{"conversation_id":{"type":"string"},"cwd":{"type":"string"}}}),
        ),
        tool(
            "detect_sandbox",
            "Detect likely sandbox restrictions and write access for the current repo and sync store",
            json!({"type":"object","properties":{"cwd":{"type":"string"}}}),
        ),
        tool(
            "create_checkpoint",
            "Create a checkpoint",
            json!({"type":"object","required":["cwd"],"properties":{"cwd":{"type":"string"},"title":{"type":"string"},"conversation_id":{"type":"string"},"summary":{"type":"string"},"last_assistant_message":{"type":"string"},"provenance":{"type":"object"}}}),
        ),
        tool(
            "record_stop_work",
            "Record a stop-work checkpoint",
            json!({"type":"object","required":["cwd"],"properties":{"cwd":{"type":"string"},"title":{"type":"string"},"conversation_id":{"type":"string"},"summary":{"type":"string"},"last_assistant_message":{"type":"string"},"provenance":{"type":"object"}}}),
        ),
        tool(
            "get_status",
            "Get agent-sync status",
            json!({"type":"object","properties":{}}),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing string param {key}"))
}

fn error(id: Option<Value>, code: i64, message: &str) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(json!({"code": code, "message": message})),
    }
}
