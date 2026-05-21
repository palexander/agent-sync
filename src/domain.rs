use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Active,
    Stale,
    Claimable,
    Diverged,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineSessionStatus {
    Active,
    Stale,
    Released,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub primary_repo: RepoState,
    pub status: ConversationStatus,
    pub current_owner_session_id: Option<String>,
    pub latest_checkpoint_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub host: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub status: ConversationStatus,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSession {
    pub id: String,
    pub conversation_id: String,
    pub hostname: String,
    pub cwd: PathBuf,
    pub origin_hook_format: Option<String>,
    pub origin_session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub model: Option<String>,
    pub status: MachineSessionStatus,
    pub lease_expires_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoState {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub remote_url: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub base_commit: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtySnapshot {
    pub staged_patch: Option<ArtifactRef>,
    pub unstaged_patch: Option<ArtifactRef>,
    pub untracked_files: Vec<FileSnapshot>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    pub content: ArtifactRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub conversation_id: String,
    pub machine_session_id: String,
    pub parent_checkpoint_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub summary: Option<String>,
    pub summary_poor: bool,
    pub last_assistant_message: Option<String>,
    pub transcript: Option<ArtifactRef>,
    pub repo: RepoState,
    pub dirty: DirtySnapshot,
    pub hostname: String,
    pub provenance: HookProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookProvenance {
    pub hook_format: Option<String>,
    pub hook_event_name: Option<String>,
    pub source_session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ConversationCreated {
        conversation: Conversation,
    },
    MachineSessionUpserted {
        session: MachineSession,
    },
    CheckpointCreated {
        checkpoint: Checkpoint,
    },
    ConversationClaimed {
        conversation_id: String,
        new_owner_session_id: String,
        previous_owner_session_id: Option<String>,
        claimed_at: DateTime<Utc>,
    },
    ConversationRepoUpdated {
        conversation_id: String,
        repo: RepoState,
        updated_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffPlan {
    pub conversation_id: String,
    pub title: String,
    pub checkpoint_id: Option<String>,
    pub resume_context: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxReport {
    pub detected: bool,
    pub indicators: Vec<String>,
    pub git_metadata_writable: Option<bool>,
    pub sync_root_writable: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeResult {
    pub conversation: ConversationSummary,
    pub handoff_plan: HandoffPlan,
    pub sandbox: SandboxReport,
    pub commands: Vec<CommandOutcome>,
    pub head: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyDirtyResult {
    pub checkpoint_id: String,
    pub applied: bool,
    pub commands: Vec<CommandOutcome>,
    pub restored_files: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub target: String,
    pub skill_installed: bool,
    pub hooks_installed: bool,
    pub backups: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub doctor: Option<HookTargetDoctorReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateReport {
    pub updated: bool,
    pub from_version: String,
    pub to_version: String,
    pub target: String,
    pub binary_path: PathBuf,
    pub commands: Vec<CommandOutcome>,
    pub install_report: Option<Vec<InstallReport>>,
    pub doctor: Option<serde_json::Value>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCheckReport {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub checked_at: DateTime<Utc>,
    pub cache_hit: bool,
    pub instructions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDoctorReport {
    pub targets: Vec<HookTargetDoctorReport>,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookTargetDoctorReport {
    pub target: String,
    pub skill_path: PathBuf,
    pub config_path: PathBuf,
    pub skill_installed: bool,
    pub config_exists: bool,
    pub config_valid_json: bool,
    pub managed_hooks_present: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateReport {
    pub sync_root: PathBuf,
    pub wrote: bool,
    pub read_back: bool,
    pub removed: bool,
    pub warnings: Vec<String>,
}
