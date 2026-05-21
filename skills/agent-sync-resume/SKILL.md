---
name: agent-sync-resume
description: Resume, list, claim, or hand off agent coding conversations across machines using the agent-sync CLI. Use when the user asks to continue a previous coding session, resume work from another computer, list recent agent sessions, or recover context from Claude Code, Codex, or Conductor.
compatibility: Requires the agent-sync binary on PATH and shell access.
---

# Agent Sync Resume

Use `agent-sync` as the source of truth. Confirm the conversation before resuming.

## Version Check

At the start of setup or resume work, run:

```bash
agent-sync version-check
```

If it reports `update_available: true`, tell the user and run:

```bash
agent-sync update
```

Then continue with the original task.

## Resume

```bash
agent-sync recent --limit 10
```

If one match is not obvious, ask the user to choose. Then inspect:

```bash
agent-sync handoff <conversation-id>
```

After confirmation:

```bash
agent-sync resume <conversation-id> --cwd "$PWD"
```

Continue from the returned `resume_context`. If `resume` reports sandbox or git failures, show the warning and ask the user to rerun with permissions.

If `handoff` warns about dirty artifacts, apply them after resume:

```bash
agent-sync apply-dirty <checkpoint-id> --cwd "$PWD"
```

Manual fallback: fetch/pull the branch, then run:

```bash
agent-sync refresh <conversation-id> --cwd "$PWD"
```

## Checkpointing

When the user asks to save, stop, checkpoint, or hand off current work, run:

```bash
agent-sync checkpoint --cwd "$PWD" --summary "<compact summary>"
```

Prefer a concise summary with goal, current state, decisions, changed files, tests, and next step. Raw transcripts and dirty patches are stored separately by hooks when available.

## Guardrails

- Do not claim a conversation without user confirmation.
- Do not silently apply patches if the handoff plan reports ambiguity.
- Keep context compact; read raw transcripts only if the summary is insufficient.
