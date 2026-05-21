# agent-sync

`agent-sync` is an app-agnostic continuity service for agentic coding sessions. It records checkpoints from Claude Code/Codex-compatible hooks and exposes a small JSON CLI that agents can use to list, hand off, and claim conversations across machines.

V1 is intentionally CLI-first. MCP support remains experimental; the recommended agent distribution path is the `skills/agent-sync-resume` Agent Skill plus hook configuration.

## Commands

```bash
agent-sync recent --limit 10
agent-sync handoff <conversation-id>
agent-sync resume <conversation-id> --cwd "$PWD"
agent-sync apply-dirty <checkpoint-id> --cwd "$PWD"
agent-sync claim <conversation-id> --cwd "$PWD"
agent-sync refresh <conversation-id> --cwd "$PWD"
agent-sync sandbox --cwd "$PWD"
agent-sync checkpoint --cwd "$PWD" --summary "compact state"
agent-sync checkpoint --new --cwd "$PWD" --title "new task" --summary "compact state"
agent-sync install all
agent-sync validate-sync
agent-sync storage
agent-sync prune --older-than 30d
agent-sync prune --older-than 30d --execute
agent-sync hook claude
agent-sync hook codex
agent-sync status
agent-sync doctor
agent-sync doctor --hooks --storage
```

All agent-facing commands print JSON.

## Install From Release

The recommended install path is the latest GitHub release binary:

```bash
curl -fsSL https://raw.githubusercontent.com/palexander/agent-sync/main/scripts/install-release.sh | bash
```

The installer detects macOS/Linux and CPU architecture, downloads the matching release tarball, verifies its SHA-256 checksum, installs `agent-sync` to `~/.local/bin`, then runs:

```bash
agent-sync install all
agent-sync doctor --hooks --storage
```

If `~/.local/bin` is not on your `PATH`, add this to your shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Supported release targets:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`

## Local Install

```bash
cargo install --path /Users/palexander/Documents/agent-sync --force
agent-sync install all
```

`install all` installs global Codex/Claude skills and merges hook entries into the user configs with timestamped backups.

For a repeatable local install:

```bash
bash /Users/palexander/Documents/agent-sync/scripts/install.sh
```

That builds the binary, installs both skills/hooks, runs hook/storage diagnostics, and validates the configured sync root.

## Release Process

CI runs formatting, Clippy, and tests on pushes to `main` and pull requests.

To publish a release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds platform tarballs, publishes checksum files, and creates a GitHub release with generated notes.

## Handoff behavior

- `resume` is the preferred agent path. It claims the conversation, fetches/pulls the branch, and refreshes stored repo state.
- `apply-dirty` restores tracked patches and non-ignored untracked file artifacts captured by a checkpoint.
- `checkpoint` auto-matches an existing conversation by repo and branch for hook continuity. Use `--new` when a human intentionally starts a distinct task on the same branch.
- `sandbox` reports whether the current agent process can write `.git` metadata and the sync root.
- `validate-sync` performs a real write/read/delete probe against the configured sync root.
- `doctor --hooks --storage` validates global skill installation, hook JSON, managed hook entries, and storage stats.
- `prune` is dry-run by default. Use `--execute` to remove unreferenced objects. `--older-than 30d` only prunes unreferenced objects older than the requested age.

## Default Store

```text
~/Library/Mobile Documents/com~apple~CloudDocs/agent-sync
```

Override with `--sync-root` or `AGENT_SYNC_ROOT`.

## Sandbox Notes

If an agent is running in a restricted sandbox, ask it to run:

```bash
agent-sync sandbox --cwd "$PWD"
```

When `.git` or the sync root is not writable, `resume` returns JSON with the commands/context it could not execute instead of mutating the repo. In that case, run `agent-sync resume ...` from an unsandboxed shell or let an unsandboxed agent perform the handoff.
