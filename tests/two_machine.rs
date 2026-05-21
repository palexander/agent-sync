use std::process::Command;

fn bin() -> String {
    env!("CARGO_BIN_EXE_agent-sync").to_string()
}

#[test]
fn host_a_checkpoint_host_b_claim_and_handoff() {
    let temp = tempfile::tempdir().unwrap();
    let sync = temp.path().join("sync");
    let cache_a = temp.path().join("cache-a");
    let cache_b = temp.path().join("cache-b");
    let cwd = temp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();

    let output = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache_a.to_str().unwrap(),
            "--hostname",
            "host-a",
            "hook",
            "codex",
        ])
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            let payload = format!(
                r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"Stop","last_assistant_message":"done"}}"#,
                cwd.display()
            );
            child.stdin.as_mut().unwrap().write_all(payload.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(output.status.success());

    let list = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache_b.to_str().unwrap(),
            "--hostname",
            "host-b",
            "status",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let status: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(status["conversations"], 1);

    let conv_dir = sync.join("registry/conversations");
    let conv_file = std::fs::read_dir(conv_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let conversation: serde_json::Value =
        serde_json::from_slice(&std::fs::read(conv_file).unwrap()).unwrap();
    let conversation_id = conversation["id"].as_str().unwrap();

    let handoff = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache_b.to_str().unwrap(),
            "--hostname",
            "host-b",
            "handoff",
            conversation_id,
        ])
        .output()
        .unwrap();
    assert!(handoff.status.success());
    let handoff_json: serde_json::Value = serde_json::from_slice(&handoff.stdout).unwrap();
    assert_eq!(handoff_json["conversation_id"], conversation_id);

    let claim = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache_b.to_str().unwrap(),
            "--hostname",
            "host-b",
            "claim",
            conversation_id,
            "--cwd",
            cwd.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(claim.status.success());
    let response: serde_json::Value = serde_json::from_slice(&claim.stdout).unwrap();
    assert_eq!(response["id"], conversation_id);
}

#[test]
fn checkpoint_new_creates_distinct_conversations() {
    let temp = tempfile::tempdir().unwrap();
    let sync = temp.path().join("sync");
    let cache = temp.path().join("cache");
    let cwd = temp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();

    for title in ["one", "two"] {
        let output = Command::new(bin())
            .args([
                "--sync-root",
                sync.to_str().unwrap(),
                "--cache-root",
                cache.to_str().unwrap(),
                "checkpoint",
                "--new",
                "--cwd",
                cwd.to_str().unwrap(),
                "--title",
                title,
                "--summary",
                title,
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    let list = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache.to_str().unwrap(),
            "recent",
            "--limit",
            "10",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let conversations: Vec<serde_json::Value> = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(conversations.len(), 2);
}

#[test]
fn prune_accepts_older_than_duration() {
    let temp = tempfile::tempdir().unwrap();
    let sync = temp.path().join("sync");
    let cache = temp.path().join("cache");
    let output = Command::new(bin())
        .args([
            "--sync-root",
            sync.to_str().unwrap(),
            "--cache-root",
            cache.to_str().unwrap(),
            "prune",
            "--older-than",
            "30d",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["older_than_seconds"], 2_592_000);
    assert_eq!(report["execute"], false);
}
