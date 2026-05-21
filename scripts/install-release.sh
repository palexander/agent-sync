#!/usr/bin/env bash
set -euo pipefail

repo="${AGENT_SYNC_REPO:-palexander/agent-sync}"
install_dir="${AGENT_SYNC_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-gnu" ;;
  *)
    echo "unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  arm64|aarch64) arch="aarch64" ;;
  x86_64|amd64) arch="x86_64" ;;
  *)
    echo "unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

target="${arch}-${os}"
archive="agent-sync-${target}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

download_url="https://github.com/${repo}/releases/latest/download/${archive}"
checksum_url="${download_url}.sha256"

mkdir -p "$install_dir"
curl -fsSL "$download_url" -o "$tmp/$archive"
curl -fsSL "$checksum_url" -o "$tmp/$archive.sha256"

(
  cd "$tmp"
  shasum -a 256 -c "$archive.sha256"
  tar -xzf "$archive"
)

install "$tmp/agent-sync" "$install_dir/agent-sync"

if ! command -v agent-sync >/dev/null 2>&1; then
  echo "installed agent-sync to $install_dir, but that directory is not on PATH" >&2
  echo "add this to your shell profile: export PATH=\"$install_dir:\$PATH\"" >&2
fi

"$install_dir/agent-sync" install all
"$install_dir/agent-sync" doctor --hooks --storage
