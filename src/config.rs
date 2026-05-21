use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sync_root: PathBuf,
    pub cache_root: PathBuf,
    pub hostname: String,
    pub lease_timeout_seconds: i64,
    pub socket_path: PathBuf,
}

impl Config {
    pub fn resolve(
        sync_root: Option<PathBuf>,
        cache_root: Option<PathBuf>,
        hostname_override: Option<String>,
    ) -> Result<Self> {
        let home = dirs::home_dir().context("could not resolve home directory")?;
        let sync_root = sync_root.unwrap_or_else(|| {
            home.join("Library")
                .join("Mobile Documents")
                .join("com~apple~CloudDocs")
                .join("agent-sync")
        });
        let cache_root = cache_root.unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| home.join(".cache"))
                .join("agent-sync")
        });
        let hostname = hostname_override.unwrap_or_else(|| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .filter(|h| !h.trim().is_empty())
                .unwrap_or_else(|| "unknown-host".to_string())
        });
        let socket_path = cache_root.join("agent-sync.sock");
        Ok(Self {
            sync_root,
            cache_root,
            hostname,
            lease_timeout_seconds: 15 * 60,
            socket_path,
        })
    }
}
