#!/usr/bin/env bash
# Build jev-router in release mode and install jev-code, jev-chat, jev-gateway and jev-claude.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="${1:-$HOME/.local/bin}"
cargo build --release --bin code --bin chat --bin gateway
install -Dm755 target/release/code "$dest/jev-code"
install -Dm755 target/release/chat "$dest/jev-chat"
install -Dm755 target/release/gateway "$dest/jev-gateway"
install -Dm755 scripts/jev-claude "$dest/jev-claude"
# The orchestrate-mode mod, loaded by jev-claude with --plugin-dir.
mods="${XDG_DATA_HOME:-$HOME/.local/share}/jev-router/mods"
mkdir -p "$mods"
rm -rf "$mods/jev-orchestrate.new"
cp -r mods/jev-orchestrate "$mods/jev-orchestrate.new"
rm -rf "$mods/jev-orchestrate"
mv "$mods/jev-orchestrate.new" "$mods/jev-orchestrate"
# Restart a running gateway so it picks up the new binary.
pkill -x jev-gateway 2>/dev/null || true
echo "installed jev-code, jev-chat, jev-gateway and jev-claude in $dest, and the jev-orchestrate mod in $mods"
