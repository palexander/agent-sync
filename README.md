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

## Global install

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
