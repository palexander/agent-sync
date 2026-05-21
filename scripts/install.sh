#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo install --path "$repo_root" --force
agent-sync install all
agent-sync doctor --hooks --storage
agent-sync validate-sync
