use crate::domain::CommandOutcome;
use crate::domain::{ArtifactRef, DirtySnapshot, FileSnapshot, RepoState};
use crate::storage::Store;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn snapshot_repo(cwd: &Path) -> RepoState {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from);
    let root_ref = root.as_deref().unwrap_or(cwd);
    let remote_url = git(root_ref, &["config", "--get", "remote.origin.url"]).ok();
    let branch = git(root_ref, &["branch", "--show-current"]).ok();
    let head = git(root_ref, &["rev-parse", "HEAD"]).ok();
    let dirty = git(root_ref, &["status", "--porcelain=v1"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    RepoState {
        cwd: cwd.to_path_buf(),
        root,
        remote_url,
        branch,
        head: head.clone(),
        base_commit: head,
        dirty,
    }
}

pub fn snapshot_dirty(store: &Store, repo: &RepoState) -> Result<DirtySnapshot> {
    let root = match repo.root.as_deref() {
        Some(root) => root,
        None => {
            return Ok(DirtySnapshot {
                staged_patch: None,
                unstaged_patch: None,
                untracked_files: Vec::new(),
                total_bytes: 0,
            })
        }
    };
    let staged_patch = patch_artifact(store, root, &["diff", "--cached"])?;
    let unstaged_patch = patch_artifact(store, root, &["diff"])?;
    let mut total_bytes = staged_patch.as_ref().map(|a| a.bytes).unwrap_or(0)
        + unstaged_patch.as_ref().map(|a| a.bytes).unwrap_or(0);
    let mut untracked_files = Vec::new();
    if let Ok(output) = git(root, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        for rel in output.split('\0').filter(|s| !s.is_empty()) {
            let path = root.join(rel);
            if path.is_file() {
                let bytes = std::fs::read(&path)?;
                let (sha256, size) = store.write_object(&bytes)?;
                total_bytes += size;
                untracked_files.push(FileSnapshot {
                    path: rel.to_string(),
                    content: ArtifactRef {
                        sha256,
                        bytes: size,
                        media_type: "application/octet-stream".to_string(),
                    },
                });
            }
        }
    }
    Ok(DirtySnapshot {
        staged_patch,
        unstaged_patch,
        untracked_files,
        total_bytes,
    })
}

fn patch_artifact(store: &Store, cwd: &Path, args: &[&str]) -> Result<Option<ArtifactRef>> {
    let patch = git_raw(cwd, args).unwrap_or_default();
    if patch.trim().is_empty() {
        return Ok(None);
    }
    let (sha256, bytes) = store.write_object(patch.as_bytes())?;
    Ok(Some(ArtifactRef {
        sha256,
        bytes,
        media_type: "text/x-diff".to_string(),
    }))
}

fn git_raw(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

pub fn run_git(cwd: &Path, args: &[&str]) -> CommandOutcome {
    let output = Command::new("git").args(args).current_dir(cwd).output();
    match output {
        Ok(output) => CommandOutcome {
            command: format!("git {}", args.join(" ")),
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_string(),
        },
        Err(err) => CommandOutcome {
            command: format!("git {}", args.join(" ")),
            success: false,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

pub fn fetch_and_pull(cwd: &Path, branch: &str) -> Vec<CommandOutcome> {
    let fetch = run_git(cwd, &["fetch", "origin", branch]);
    if !fetch.success {
        return vec![fetch];
    }
    let pull = run_git(cwd, &["pull", "--ff-only", "origin", branch]);
    vec![fetch, pull]
}

pub fn apply_patch(cwd: &Path, patch_path: &Path) -> CommandOutcome {
    let path = patch_path.to_string_lossy();
    run_git(cwd, &["apply", "--3way", &path])
}

pub fn is_clean(cwd: &Path) -> bool {
    git(cwd, &["status", "--porcelain=v1"])
        .map(|status| status.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_git_directory_has_empty_repo_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let repo = snapshot_repo(dir.path());
        assert!(repo.root.is_none());
        assert!(!repo.dirty);
    }
}
