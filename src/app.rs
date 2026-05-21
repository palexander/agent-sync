use crate::config::Config;
use crate::domain::*;
use crate::git;
use crate::storage::Store;
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInput {
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub new_conversation: bool,
    pub summary: Option<String>,
    pub last_assistant_message: Option<String>,
    pub provenance: HookProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub hostname: String,
    pub sync_root: PathBuf,
    pub cache_root: PathBuf,
    pub conversations: usize,
    pub latest_updated_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AgentSync {
    pub config: Config,
    store: Store,
}

impl AgentSync {
    pub fn new(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.cache_root)?;
        let store = Store::new(config.sync_root.clone())?;
        Ok(Self { config, store })
    }

    pub fn status(&self) -> Result<Status> {
        let conversations = self.store.list_conversations()?;
        Ok(Status {
            hostname: self.config.hostname.clone(),
            sync_root: self.config.sync_root.clone(),
            cache_root: self.config.cache_root.clone(),
            conversations: conversations.len(),
            latest_updated_at: conversations.first().map(|c| c.updated_at),
        })
    }

    pub fn doctor(&self) -> Result<String> {
        let mut lines = Vec::new();
        lines.push(format!("hostname: {}", self.config.hostname));
        lines.push(format!("sync_root: {}", self.config.sync_root.display()));
        lines.push(format!("cache_root: {}", self.config.cache_root.display()));
        lines.push(format!(
            "sync_root_exists: {}",
            self.config.sync_root.exists()
        ));
        lines.push(format!(
            "socket_path: {}",
            self.config.socket_path.display()
        ));
        lines.push(format!(
            "conversation_count: {}",
            self.store.list_conversations()?.len()
        ));
        let stats = self.store.storage_stats()?;
        lines.push(format!("storage_files: {}", stats.files));
        lines.push(format!("storage_bytes: {}", stats.bytes));
        Ok(lines.join("\n"))
    }

    pub fn list_recent_conversations(&self, limit: usize) -> Result<Vec<Conversation>> {
        let mut conversations = self.store.list_conversations()?;
        conversations.truncate(limit);
        Ok(conversations)
    }

    pub fn list_recent_summaries(&self, limit: usize) -> Result<Vec<ConversationSummary>> {
        self.list_recent_conversations(limit)?
            .into_iter()
            .map(|conversation| self.summarize_conversation(conversation))
            .collect()
    }

    pub fn get_conversation(&self, id: &str) -> Result<Conversation> {
        self.store.read_conversation(id)
    }

    pub fn create_checkpoint(&self, input: CheckpointInput) -> Result<Checkpoint> {
        let now = Utc::now();
        let repo = git::snapshot_repo(&input.cwd);
        let conversation = if input.new_conversation {
            None
        } else {
            match input.conversation_id.as_deref() {
                Some(id) => self.store.read_conversation(id).ok(),
                None => self.match_conversation(&repo)?,
            }
        }
        .unwrap_or_else(|| Conversation {
            id: format!("conv_{}", Uuid::new_v4()),
            title: input
                .title
                .clone()
                .or_else(|| repo.branch.clone())
                .unwrap_or_else(|| "Untitled agent session".to_string()),
            primary_repo: repo.clone(),
            status: ConversationStatus::Active,
            current_owner_session_id: None,
            latest_checkpoint_id: None,
            created_at: now,
            updated_at: now,
        });

        if conversation.latest_checkpoint_id.is_none() {
            self.store.append_event(
                &conversation.id,
                &Event::ConversationCreated {
                    conversation: conversation.clone(),
                },
            )?;
        }

        let session_id = format!("sess_{}", Uuid::new_v4());
        let session = MachineSession {
            id: session_id.clone(),
            conversation_id: conversation.id.clone(),
            hostname: self.config.hostname.clone(),
            cwd: input.cwd.clone(),
            origin_hook_format: input.provenance.hook_format.clone(),
            origin_session_id: input.provenance.source_session_id.clone(),
            transcript_path: input.provenance.transcript_path.clone(),
            model: input.provenance.model.clone(),
            status: MachineSessionStatus::Active,
            lease_expires_at: now + Duration::seconds(self.config.lease_timeout_seconds),
            last_heartbeat_at: now,
        };

        let transcript = input
            .provenance
            .transcript_path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| {
                let (sha256, size) = self.store.write_object(&bytes)?;
                Ok::<_, anyhow::Error>(ArtifactRef {
                    sha256,
                    bytes: size,
                    media_type: "application/jsonl".to_string(),
                })
            })
            .transpose()?;

        let dirty = git::snapshot_dirty(&self.store, &repo)?;
        let checkpoint = Checkpoint {
            id: format!("chk_{}", Uuid::new_v4()),
            conversation_id: conversation.id.clone(),
            machine_session_id: session.id.clone(),
            parent_checkpoint_id: conversation.latest_checkpoint_id.clone(),
            created_at: now,
            summary_poor: input
                .summary
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty(),
            summary: input.summary.clone(),
            last_assistant_message: input.last_assistant_message,
            transcript,
            repo: repo.clone(),
            dirty,
            hostname: self.config.hostname.clone(),
            provenance: input.provenance,
        };

        let mut updated = conversation;
        updated.status = ConversationStatus::Active;
        updated.current_owner_session_id = Some(session.id.clone());
        updated.latest_checkpoint_id = Some(checkpoint.id.clone());
        updated.updated_at = now;
        updated.primary_repo = repo;

        self.store.append_event(
            &updated.id,
            &Event::MachineSessionUpserted {
                session: session.clone(),
            },
        )?;
        self.store.append_event(
            &updated.id,
            &Event::CheckpointCreated {
                checkpoint: checkpoint.clone(),
            },
        )?;
        self.store.write_conversation(&updated)?;
        Ok(checkpoint)
    }

    pub fn claim_conversation(&self, id: &str, cwd: PathBuf) -> Result<Conversation> {
        let now = Utc::now();
        let mut conversation = self.store.read_conversation(id)?;
        let previous = conversation.current_owner_session_id.clone();
        let session_id = format!("sess_{}", Uuid::new_v4());
        let repo = git::snapshot_repo(&cwd);
        let session = MachineSession {
            id: session_id.clone(),
            conversation_id: id.to_string(),
            hostname: self.config.hostname.clone(),
            cwd,
            origin_hook_format: None,
            origin_session_id: None,
            transcript_path: None,
            model: None,
            status: MachineSessionStatus::Active,
            lease_expires_at: now + Duration::seconds(self.config.lease_timeout_seconds),
            last_heartbeat_at: now,
        };
        conversation.status = ConversationStatus::Active;
        conversation.current_owner_session_id = Some(session_id.clone());
        conversation.updated_at = now;
        self.store
            .append_event(id, &Event::MachineSessionUpserted { session })?;
        self.store.append_event(
            id,
            &Event::ConversationClaimed {
                conversation_id: id.to_string(),
                new_owner_session_id: session_id,
                previous_owner_session_id: previous,
                claimed_at: now,
            },
        )?;
        if repo.remote_url.is_some() {
            conversation.primary_repo = repo;
        }
        self.store.write_conversation(&conversation)?;
        Ok(conversation)
    }

    pub fn refresh_conversation_repo(&self, id: &str, cwd: PathBuf) -> Result<Conversation> {
        let now = Utc::now();
        let mut conversation = self.store.read_conversation(id)?;
        let repo = git::snapshot_repo(&cwd);
        if repo.remote_url.is_some() {
            conversation.primary_repo = repo.clone();
            conversation.updated_at = now;
            self.store.append_event(
                id,
                &Event::ConversationRepoUpdated {
                    conversation_id: id.to_string(),
                    repo,
                    updated_at: now,
                },
            )?;
            self.store.write_conversation(&conversation)?;
        }
        Ok(conversation)
    }

    pub fn resume_conversation(&self, id: &str, cwd: PathBuf) -> Result<ResumeResult> {
        let handoff_plan = self.get_handoff_plan(id)?;
        let sandbox = self.detect_sandbox(Some(cwd.clone()))?;
        if !sandbox.sync_root_writable || sandbox.git_metadata_writable == Some(false) {
            return Ok(ResumeResult {
                conversation: self.summarize_conversation(self.store.read_conversation(id)?)?,
                handoff_plan,
                sandbox,
                commands: Vec::new(),
                head: git::snapshot_repo(&cwd).head,
            });
        }
        let conversation = self.claim_conversation(id, cwd.clone())?;
        let branch = self
            .latest_checkpoint(id)?
            .and_then(|checkpoint| checkpoint.repo.branch)
            .or_else(|| conversation.primary_repo.branch.clone())
            .unwrap_or_else(|| "main".to_string());
        let commands = git::fetch_and_pull(&cwd, &branch);
        if commands.iter().all(|outcome| outcome.success) {
            self.refresh_conversation_repo(id, cwd.clone())?;
        }
        let repo_after = git::snapshot_repo(&cwd);
        let conversation = self.store.read_conversation(id)?;
        Ok(ResumeResult {
            conversation: self.summarize_conversation(conversation)?,
            handoff_plan,
            sandbox,
            commands,
            head: repo_after.head,
        })
    }

    pub fn detect_sandbox(&self, cwd: Option<PathBuf>) -> Result<SandboxReport> {
        Ok(detect_sandbox_for_config(&self.config, cwd))
    }

    pub fn apply_dirty(
        &self,
        checkpoint_id: &str,
        cwd: PathBuf,
        force: bool,
    ) -> Result<ApplyDirtyResult> {
        let checkpoint = self
            .store
            .find_checkpoint(checkpoint_id)?
            .with_context(|| format!("checkpoint not found: {checkpoint_id}"))?;
        let mut warnings = Vec::new();
        if !force && !git::is_clean(&cwd) {
            warnings.push(
                "worktree is not clean; rerun with --force to apply dirty artifacts".to_string(),
            );
            return Ok(ApplyDirtyResult {
                checkpoint_id: checkpoint_id.to_string(),
                applied: false,
                commands: Vec::new(),
                restored_files: Vec::new(),
                warnings,
            });
        }
        let temp = tempfile_path("agent-sync-dirty");
        std::fs::create_dir_all(&temp)?;
        let mut commands = Vec::new();
        if let Some(patch) = &checkpoint.dirty.staged_patch {
            let path = temp.join("staged.patch");
            std::fs::write(&path, self.store.read_object(&patch.sha256)?)?;
            commands.push(git::apply_patch(&cwd, &path));
        }
        if let Some(patch) = &checkpoint.dirty.unstaged_patch {
            let path = temp.join("unstaged.patch");
            std::fs::write(&path, self.store.read_object(&patch.sha256)?)?;
            commands.push(git::apply_patch(&cwd, &path));
        }
        let mut restored_files = Vec::new();
        for file in &checkpoint.dirty.untracked_files {
            let target = cwd.join(&file.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, self.store.read_object(&file.content.sha256)?)?;
            restored_files.push(file.path.clone());
        }
        let applied = commands.iter().all(|command| command.success);
        Ok(ApplyDirtyResult {
            checkpoint_id: checkpoint_id.to_string(),
            applied,
            commands,
            restored_files,
            warnings,
        })
    }

    pub fn validate_sync(&self) -> Result<ValidateReport> {
        let path = self
            .config
            .sync_root
            .join(format!(".agent-sync-validate-{}", Uuid::new_v4()));
        let bytes = b"agent-sync validate";
        std::fs::write(&path, bytes)?;
        let read_back = std::fs::read(&path)
            .map(|read| read == bytes)
            .unwrap_or(false);
        let removed = std::fs::remove_file(&path).is_ok();
        Ok(ValidateReport {
            sync_root: self.config.sync_root.clone(),
            wrote: true,
            read_back,
            removed,
            warnings: Vec::new(),
        })
    }

    pub fn storage_stats(&self) -> Result<crate::storage::StorageStats> {
        self.store.storage_stats()
    }

    pub fn prune(
        &self,
        execute: bool,
        older_than: Option<std::time::Duration>,
    ) -> Result<crate::storage::PruneReport> {
        self.store.prune_objects(execute, older_than)
    }

    pub fn get_handoff_plan(&self, id: &str) -> Result<HandoffPlan> {
        let conversation = self.store.read_conversation(id)?;
        let checkpoint = self.latest_checkpoint(id)?;
        let effective_status = self.effective_status(&conversation)?;
        let mut warnings = Vec::new();
        if effective_status != ConversationStatus::Active {
            warnings.push(format!(
                "conversation is currently {} based on owner lease state",
                status_name(&effective_status)
            ));
        }
        if checkpoint.as_ref().map(|c| c.summary_poor).unwrap_or(true) {
            warnings.push("latest checkpoint has no agent-generated summary".to_string());
        }
        if checkpoint
            .as_ref()
            .map(|c| c.dirty.total_bytes > 0)
            .unwrap_or(false)
        {
            warnings.push("latest checkpoint contains dirty patch artifacts".to_string());
        }
        let resume_context = match checkpoint.as_ref() {
            Some(c) => format!(
                "Conversation: {}\nStatus: {}\nCheckpoint: {}\nHost: {}\nRepo: {}\nBranch: {}\nHead: {}\nSummary: {}\nLast: {}",
                conversation.title,
                status_name(&effective_status),
                c.id,
                c.hostname,
                c.repo.remote_url.clone().unwrap_or_else(|| "unknown".to_string()),
                c.repo.branch.clone().unwrap_or_else(|| "unknown".to_string()),
                c.repo.head.clone().unwrap_or_else(|| "unknown".to_string()),
                c.summary.clone().unwrap_or_else(|| "(no summary available)".to_string()),
                c.last_assistant_message.clone().unwrap_or_else(|| "(none)".to_string())
            ),
            None => format!(
                "Conversation: {}\nStatus: {}\nNo checkpoint is available.",
                conversation.title,
                status_name(&effective_status)
            ),
        };
        Ok(HandoffPlan {
            conversation_id: conversation.id,
            title: conversation.title,
            checkpoint_id: checkpoint.map(|c| c.id),
            resume_context,
            warnings,
        })
    }

    fn summarize_conversation(&self, conversation: Conversation) -> Result<ConversationSummary> {
        let checkpoint = self.latest_checkpoint(&conversation.id)?;
        let host = checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.hostname.clone());
        let status = self.effective_status(&conversation)?;
        Ok(ConversationSummary {
            id: conversation.id,
            title: conversation.title,
            host,
            repo: conversation.primary_repo.remote_url,
            branch: conversation.primary_repo.branch,
            head: conversation.primary_repo.head,
            status,
            updated_at: conversation.updated_at,
        })
    }

    fn latest_checkpoint(&self, conversation_id: &str) -> Result<Option<Checkpoint>> {
        let mut latest = None;
        for event in self.store.read_events(conversation_id)? {
            if let Event::CheckpointCreated { checkpoint } = event {
                latest = Some(checkpoint);
            }
        }
        Ok(latest)
    }

    fn effective_status(&self, conversation: &Conversation) -> Result<ConversationStatus> {
        if matches!(
            conversation.status,
            ConversationStatus::Archived | ConversationStatus::Diverged
        ) {
            return Ok(conversation.status.clone());
        }

        let Some(owner_session_id) = &conversation.current_owner_session_id else {
            return Ok(ConversationStatus::Claimable);
        };

        let Some(owner_session) = self.machine_session(&conversation.id, owner_session_id)? else {
            return Ok(ConversationStatus::Stale);
        };

        if owner_session.status != MachineSessionStatus::Active {
            return Ok(ConversationStatus::Stale);
        }

        if owner_session.lease_expires_at <= Utc::now() {
            return Ok(ConversationStatus::Stale);
        }

        Ok(ConversationStatus::Active)
    }

    fn machine_session(
        &self,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<Option<MachineSession>> {
        let mut matched = None;
        for event in self.store.read_events(conversation_id)? {
            if let Event::MachineSessionUpserted { session } = event {
                if session.id == session_id {
                    matched = Some(session);
                }
            }
        }
        Ok(matched)
    }

    fn match_conversation(&self, repo: &RepoState) -> Result<Option<Conversation>> {
        for conversation in self.store.list_conversations()? {
            if conversation.primary_repo.remote_url == repo.remote_url
                && conversation.primary_repo.branch == repo.branch
                && repo.remote_url.is_some()
            {
                return Ok(Some(conversation));
            }
        }
        Ok(None)
    }

    #[cfg(test)]
    pub fn rebuild_from_events(
        &self,
        id: &str,
    ) -> Result<std::collections::HashMap<String, usize>> {
        let events = self.store.read_events(id)?;
        let mut counts = std::collections::HashMap::new();
        for event in events {
            let key = match event {
                Event::ConversationCreated { .. } => "conversation_created",
                Event::MachineSessionUpserted { .. } => "machine_session_upserted",
                Event::CheckpointCreated { .. } => "checkpoint_created",
                Event::ConversationClaimed { .. } => "conversation_claimed",
                Event::ConversationRepoUpdated { .. } => "conversation_repo_updated",
            };
            *counts.entry(key.to_string()).or_insert(0) += 1;
        }
        Ok(counts)
    }
}

fn tempfile_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()))
}

fn status_name(status: &ConversationStatus) -> &'static str {
    match status {
        ConversationStatus::Active => "active",
        ConversationStatus::Stale => "stale",
        ConversationStatus::Claimable => "claimable",
        ConversationStatus::Diverged => "diverged",
        ConversationStatus::Archived => "archived",
    }
}

pub fn detect_sandbox_for_config(config: &Config, cwd: Option<PathBuf>) -> SandboxReport {
    let mut indicators = Vec::new();
    for key in [
        "CODEX_SANDBOX",
        "CODEX_SANDBOX_NETWORK_DISABLED",
        "CODEX_SHELL",
        "CLAUDE_CODE_SANDBOX",
    ] {
        if let Ok(value) = std::env::var(key) {
            indicators.push(format!("{key}={value}"));
        }
    }
    let sync_root_writable = probe_dir_writable(&config.sync_root);
    let git_metadata_writable = cwd.and_then(|cwd| {
        let repo = git::snapshot_repo(&cwd);
        repo.root
            .as_ref()
            .map(|root| probe_dir_writable(&root.join(".git")))
    });
    let mut warnings = Vec::new();
    if !sync_root_writable {
        warnings
            .push("sync root is not writable; checkpoint and claim writes may fail".to_string());
    }
    if git_metadata_writable == Some(false) {
        warnings.push(
            "git metadata is not writable; fetch, pull, commit, and apply may fail".to_string(),
        );
    }
    SandboxReport {
        detected: !indicators.is_empty(),
        indicators,
        git_metadata_writable,
        sync_root_writable,
        warnings,
    }
}

fn probe_dir_writable(dir: &std::path::Path) -> bool {
    if !dir.exists() {
        return false;
    }
    let path = dir.join(format!(".agent-sync-probe-{}", Uuid::new_v4()));
    match std::fs::write(&path, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(path);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> AgentSync {
        let dir = tempfile::tempdir().unwrap().keep();
        let config = Config::resolve(
            Some(dir.join("sync")),
            Some(dir.join("cache")),
            Some("host-a".into()),
        )
        .unwrap();
        AgentSync::new(config).unwrap()
    }

    fn test_app_with_lease_timeout(lease_timeout_seconds: i64) -> AgentSync {
        let dir = tempfile::tempdir().unwrap().keep();
        let mut config = Config::resolve(
            Some(dir.join("sync")),
            Some(dir.join("cache")),
            Some("host-a".into()),
        )
        .unwrap();
        config.lease_timeout_seconds = lease_timeout_seconds;
        AgentSync::new(config).unwrap()
    }

    #[test]
    fn claim_marks_new_owner() {
        let app = test_app();
        let cwd = tempfile::tempdir().unwrap();
        let checkpoint = app
            .create_checkpoint(CheckpointInput {
                cwd: cwd.path().to_path_buf(),
                title: Some("test".into()),
                conversation_id: None,
                new_conversation: false,
                summary: Some("summary".into()),
                last_assistant_message: None,
                provenance: HookProvenance::default(),
            })
            .unwrap();
        let before = app.get_conversation(&checkpoint.conversation_id).unwrap();
        let claimed = app
            .claim_conversation(&checkpoint.conversation_id, cwd.path().to_path_buf())
            .unwrap();
        assert_ne!(
            before.current_owner_session_id,
            claimed.current_owner_session_id
        );
    }

    #[test]
    fn rebuild_counts_events() {
        let app = test_app();
        let cwd = tempfile::tempdir().unwrap();
        let checkpoint = app
            .create_checkpoint(CheckpointInput {
                cwd: cwd.path().to_path_buf(),
                title: Some("test".into()),
                conversation_id: None,
                new_conversation: false,
                summary: Some("summary".into()),
                last_assistant_message: None,
                provenance: HookProvenance::default(),
            })
            .unwrap();
        let counts = app
            .rebuild_from_events(&checkpoint.conversation_id)
            .unwrap();
        assert_eq!(counts.get("checkpoint_created"), Some(&1));
    }

    #[test]
    fn summaries_derive_stale_status_from_expired_owner_lease() {
        let app = test_app_with_lease_timeout(-1);
        let cwd = tempfile::tempdir().unwrap();
        let checkpoint = app
            .create_checkpoint(CheckpointInput {
                cwd: cwd.path().to_path_buf(),
                title: Some("test".into()),
                conversation_id: None,
                new_conversation: false,
                summary: Some("summary".into()),
                last_assistant_message: None,
                provenance: HookProvenance::default(),
            })
            .unwrap();

        let stored = app.get_conversation(&checkpoint.conversation_id).unwrap();
        assert_eq!(stored.status, ConversationStatus::Active);

        let summary = app.list_recent_summaries(1).unwrap().remove(0);
        assert_eq!(summary.status, ConversationStatus::Stale);

        let handoff = app.get_handoff_plan(&checkpoint.conversation_id).unwrap();
        assert!(handoff.resume_context.contains("Status: stale"));
        assert!(handoff
            .warnings
            .iter()
            .any(|warning| warning.contains("currently stale")));
    }
}
