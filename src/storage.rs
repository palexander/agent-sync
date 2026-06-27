use crate::domain::{Checkpoint, Conversation, Event, RepoState};
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Build the lookup key for the conversation index from a repo's identity.
/// Returns `None` when the repo has no remote, since such conversations are
/// never matched by `match_conversation` and must not be indexed.
pub fn repo_index_key(repo: &RepoState) -> Option<String> {
    let url = repo.remote_url.as_ref()?;
    // `\n` is a safe separator: it cannot appear in a git remote URL or branch.
    Some(format!("{}\n{}", url, repo.branch.as_deref().unwrap_or("")))
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        let store = Self { root };
        store.ensure_dirs()?;
        Ok(store)
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for path in [
            self.root.join("events"),
            self.root.join("registry/conversations"),
            self.root.join("registry/machines"),
            self.root.join("objects/sha256"),
            self.root.join("manifests"),
        ] {
            fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
        }
        Ok(())
    }

    pub fn write_object(&self, bytes: &[u8]) -> Result<(String, u64)> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let (prefix, rest) = hash.split_at(2);
        let dir = self.root.join("objects/sha256").join(prefix);
        fs::create_dir_all(&dir)?;
        let path = dir.join(rest);
        if !path.exists() {
            fs::write(&path, bytes)?;
        }
        Ok((hash, bytes.len() as u64))
    }

    pub fn read_object(&self, hash: &str) -> Result<Vec<u8>> {
        let (prefix, rest) = hash.split_at(2);
        Ok(fs::read(
            self.root.join("objects/sha256").join(prefix).join(rest),
        )?)
    }

    pub fn append_event(&self, conversation_id: &str, event: &Event) -> Result<()> {
        let path = self
            .root
            .join("events")
            .join(format!("{conversation_id}.jsonl"));
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, event)?;
        writeln!(file)?;
        Ok(())
    }

    pub fn write_conversation(&self, conversation: &Conversation) -> Result<()> {
        self.write_json(
            &self
                .root
                .join("registry/conversations")
                .join(format!("{}.json", conversation.id)),
            conversation,
        )?;
        // Keep the index current so `match_conversation` stays O(1) and never
        // has to scan the whole registry (which blocks on cloud-only files).
        if let Some(key) = repo_index_key(&conversation.primary_repo) {
            self.index_upsert(&key, &conversation.id)?;
        }
        Ok(())
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("registry/index.json")
    }

    /// Read the `(remote_url, branch) -> conversation_id` index. Returns an
    /// empty map when the index does not yet exist.
    pub fn read_index(&self) -> Result<HashMap<String, String>> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        self.read_json(&path)
    }

    /// Look up a single conversation id by index key without touching the rest
    /// of the registry. This is the hot path used on every checkpoint.
    pub fn index_lookup(&self, key: &str) -> Result<Option<String>> {
        Ok(self.read_index()?.get(key).cloned())
    }

    fn index_upsert(&self, key: &str, conversation_id: &str) -> Result<()> {
        let mut index = self.read_index()?;
        if index.get(key).map(String::as_str) == Some(conversation_id) {
            return Ok(());
        }
        index.insert(key.to_string(), conversation_id.to_string());
        self.write_json(&self.index_path(), &index)
    }

    /// Rebuild the index from scratch by scanning every conversation once.
    /// Used as a one-time migration; this is the only place that still pays the
    /// full-registry read cost, and it is off the hot path.
    pub fn rebuild_index(&self) -> Result<usize> {
        let mut index = HashMap::new();
        for conversation in self.list_conversations()? {
            if let Some(key) = repo_index_key(&conversation.primary_repo) {
                index.insert(key, conversation.id.clone());
            }
        }
        let count = index.len();
        self.write_json(&self.index_path(), &index)?;
        Ok(count)
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let dir = self.root.join("registry/conversations");
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(self.read_json(&entry.path())?);
            }
        }
        out.sort_by_key(|c: &Conversation| c.updated_at);
        out.reverse();
        Ok(out)
    }

    pub fn read_conversation(&self, id: &str) -> Result<Conversation> {
        self.read_json(
            &self
                .root
                .join("registry/conversations")
                .join(format!("{id}.json")),
        )
    }

    pub fn read_events(&self, conversation_id: &str) -> Result<Vec<Event>> {
        let path = self
            .root
            .join("events")
            .join(format!("{conversation_id}.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path)?;
        let mut events = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if !line.trim().is_empty() {
                events.push(serde_json::from_str(&line)?);
            }
        }
        Ok(events)
    }

    pub fn find_checkpoint(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>> {
        for conversation in self.list_conversations()? {
            for event in self.read_events(&conversation.id)? {
                if let Event::CheckpointCreated { checkpoint } = event {
                    if checkpoint.id == checkpoint_id {
                        return Ok(Some(checkpoint));
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn storage_stats(&self) -> Result<StorageStats> {
        let mut files = 0u64;
        let mut bytes = 0u64;
        for entry in walkdir::WalkDir::new(&self.root) {
            let entry = entry?;
            if entry.file_type().is_file() {
                files += 1;
                bytes += entry.metadata()?.len();
            }
        }
        Ok(StorageStats {
            root: self.root.clone(),
            files,
            bytes,
        })
    }

    pub fn prune_objects(
        &self,
        execute: bool,
        older_than: Option<std::time::Duration>,
    ) -> Result<PruneReport> {
        let mut referenced = std::collections::HashSet::new();
        for conversation in self.list_conversations()? {
            for event in self.read_events(&conversation.id)? {
                if let Event::CheckpointCreated { checkpoint } = event {
                    collect_checkpoint_refs(&checkpoint, &mut referenced);
                }
            }
        }
        let objects_dir = self.root.join("objects/sha256");
        let mut pruned = 0u64;
        let mut bytes = 0u64;
        if objects_dir.exists() {
            for entry in walkdir::WalkDir::new(&objects_dir).min_depth(2) {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let Some(parent) = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                else {
                    continue;
                };
                let Some(rest) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let hash = format!("{parent}{rest}");
                if !referenced.contains(&hash) {
                    if let Some(older_than) = older_than {
                        let modified = entry.metadata()?.modified()?;
                        let age = modified.elapsed().unwrap_or_default();
                        if age < older_than {
                            continue;
                        }
                    }
                    pruned += 1;
                    bytes += entry.metadata()?.len();
                    if execute {
                        fs::remove_file(path)?;
                    }
                }
            }
        }
        Ok(PruneReport {
            execute,
            older_than_seconds: older_than.map(|duration| duration.as_secs()),
            pruned_objects: pruned,
            pruned_bytes: bytes,
        })
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }

    fn read_json<T: DeserializeOwned>(&self, path: &Path) -> Result<T> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageStats {
    pub root: PathBuf,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PruneReport {
    pub execute: bool,
    pub older_than_seconds: Option<u64>,
    pub pruned_objects: u64,
    pub pruned_bytes: u64,
}

fn collect_checkpoint_refs(checkpoint: &Checkpoint, refs: &mut std::collections::HashSet<String>) {
    if let Some(artifact) = &checkpoint.transcript {
        refs.insert(artifact.sha256.clone());
    }
    if let Some(artifact) = &checkpoint.dirty.staged_patch {
        refs.insert(artifact.sha256.clone());
    }
    if let Some(artifact) = &checkpoint.dirty.unstaged_patch {
        refs.insert(artifact.sha256.clone());
    }
    for file in &checkpoint.dirty.untracked_files {
        refs.insert(file.content.sha256.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_content_addressed_objects() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        let (hash, bytes) = store.write_object(b"hello").unwrap();
        assert_eq!(bytes, 5);
        assert_eq!(store.read_object(&hash).unwrap(), b"hello");
    }
}
