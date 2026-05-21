mod app;
mod cli;
mod config;
mod daemon;
mod domain;
mod git;
mod hooks;
mod install;
mod mcp;
mod storage;

#[tokio::main]
async fn main() {
    if let Err(err) = cli::run().await {
        eprintln!(
            "{}",
            serde_json::json!({
                "ok": false,
                "error": format!("{err:#}")
            })
        );
        std::process::exit(1);
    }
}
