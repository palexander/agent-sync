# agent-sync

`agent-sync` helps coding agents resume work across machines.

It records lightweight conversation checkpoints from Claude Code and Codex hooks, stores them in a synced folder, and gives agents a small JSON CLI for listing recent sessions, claiming a session, pulling the intended branch, and restoring captured dirty work.

Conductor is supported when it runs Claude Code or Codex under the hood and emits one of those hook formats.

## What It Does

- Tracks app-agnostic coding conversations across machines.
- Captures checkpoints from Claude Code and Codex hook payloads.
- Stores checkpoint summaries, repo metadata, transcripts when available, tracked patches, and non-ignored untracked files.
- Lets an agent list recent sessions and build a compact handoff plan.
- Lets an agent claim/resume a session, fetch/pull the intended branch, and refresh stored repo state.
- Provides an explicit dirty restore command for checkpoint patch artifacts.
- Installs global Codex and Claude skills/hooks so setup is not per-repo.

`agent-sync` does not mutate native Claude Code, Codex, or Conductor conversation databases. Resumption happens through the CLI and installed agent skill.

## Storage

By default, checkpoints are stored in iCloud Drive:

```text
~/Library/Mobile Documents/com~apple~CloudDocs/agent-sync
```

Override the store with either:

```bash
export AGENT_SYNC_ROOT="$HOME/path/to/synced/agent-sync"
```

or pass `--sync-root` to any command.

The store uses append-only JSONL event logs plus content-addressed immutable objects for transcripts, patches, and file snapshots. There is no synced SQLite database.

## Install

Install the latest release binary:

```bash
curl -fsSL https://raw.githubusercontent.com/palexander/agent-sync/main/scripts/install-release.sh | bash
```

The installer:

1. Detects OS and CPU architecture.
2. Downloads the matching release tarball.
3. Verifies the SHA-256 checksum.
4. Installs `agent-sync` to `~/.local/bin`.
5. Installs global Codex and Claude skills/hooks.
6. Runs hook and storage diagnostics.

If `~/.local/bin` is not on your `PATH`, add this to your shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Supported release targets:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`

Linux users should set `AGENT_SYNC_ROOT` to a real synced folder, because the default path is the macOS iCloud Drive location.

## Install From Source

```bash
git clone https://github.com/palexander/agent-sync.git
cd agent-sync
cargo install --path . --force
agent-sync install all
agent-sync doctor --hooks --storage
```

For a source checkout, the helper script does the same build/install/diagnostic flow:

```bash
bash scripts/install.sh
```

## Agent Workflow

After installation, ask your agent to use the `agent-sync-resume` skill.

Typical prompts:

```text
List my recent agent-sync sessions.
```

```text
Resume the session I was working on from my MacBook.
```

```text
Resume conversation conv_... in this repo.
```

The agent should:

1. Run `agent-sync recent` to list candidates.
2. Ask you to confirm the conversation.
3. Run `agent-sync handoff <conversation-id>` to get compact context.
4. Run `agent-sync resume <conversation-id> --cwd "$PWD"` to claim and refresh the repo.
5. Run `agent-sync apply-dirty <checkpoint-id> --cwd "$PWD"` if the handoff includes dirty artifacts and you want them restored.

## Commands

Most operational commands print JSON. `doctor` prints a compact text report by default; use `doctor --hooks --storage` for JSON diagnostics.

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
agent-sync update
agent-sync version-check
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

Key commands:

- `recent`: Lists recent conversations with id, title, host, repo, branch, head, and status.
- `handoff`: Builds compact resume context and warnings for one conversation.
- `resume`: Claims the conversation, fetches/pulls the branch from origin, and refreshes stored repo metadata.
- `apply-dirty`: Applies tracked checkpoint patches and restores non-ignored untracked files captured by a checkpoint.
- `checkpoint`: Creates a checkpoint manually. By default it auto-matches an existing conversation by repo and branch; use `--new` to force a distinct conversation.
- `sandbox`: Reports whether the process can write the sync root and local `.git` metadata.
- `install`: Installs global agent skills and hooks for `codex`, `claude`, or `all`.
- `update`: Downloads the latest release, verifies its checksum, replaces the installed binary, reruns `install all`, and returns JSON diagnostics.
- `version-check`: Checks the latest GitHub release at most once per hour and reports whether `agent-sync update` should be run.
- `doctor --hooks --storage`: Validates hook config, skill installation, and storage visibility.
- `validate-sync`: Performs a write/read/delete probe against the configured sync root.
- `prune`: Dry-runs removal of unreferenced objects. Add `--execute` to delete them.

## Hook Behavior

`agent-sync install all` installs managed hook entries for Codex and Claude. Existing hook files are backed up before they are modified.

Only stop/compaction-style hook events create checkpoints. Other installed hook entries are compatibility hooks and do not create extra checkpoints.

Hooks are fail-open: hook failures should not block the source agent.

## Dirty Work

Checkpoints capture:

- staged tracked changes as patches
- unstaged tracked changes as patches
- non-ignored untracked files as file artifacts

Dirty artifacts are not applied automatically during `resume`. Use:

```bash
agent-sync apply-dirty <checkpoint-id> --cwd "$PWD"
```

`apply-dirty` refuses to modify a dirty worktree unless you pass `--force`.

## Sandbox Notes

Some agent environments restrict filesystem writes or Git metadata updates. Check the current process with:

```bash
agent-sync sandbox --cwd "$PWD"
```

If `.git` or the sync root is not writable, `resume` returns JSON describing the blocked operation instead of mutating the repo.

## Release Process

CI runs formatting, Clippy, and tests on pushes to `main` and pull requests.

To publish a release:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

The release workflow builds platform tarballs, publishes SHA-256 checksum files, and creates a GitHub release with generated notes.
