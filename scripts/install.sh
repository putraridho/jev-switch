#!/usr/bin/env bash
# Build jev-router (optimised, incremental `local` profile) and install jev-code, jev-ask, jev-chat, jev-gateway,
# jev-claude and the delegate tool that jev-code's orchestration needs.
#
#   scripts/install.sh [dest]        dest defaults to ~/.local/bin
#
# The delegate tool is installed as `jev-delegate-mcp` next to `jev-code`, which is where
# `/orchestrate` looks for it (orchestrate::beside); JEV_DELEGATE_BIN overrides the path.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="${1:-$HOME/.local/bin}"
# The `local` profile is release with incremental builds: a reinstall after a small change
# takes seconds instead of a full optimised rebuild.
cargo build --profile local --bin code --bin ask --bin chat --bin gateway --bin delegate_mcp
install -Dm755 target/local/code "$dest/jev-code"
install -Dm755 target/local/ask "$dest/jev-ask"
install -Dm755 target/local/delegate_mcp "$dest/jev-delegate-mcp"
install -Dm755 target/local/chat "$dest/jev-chat"
install -Dm755 target/local/gateway "$dest/jev-gateway"
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
echo "installed jev-code, jev-ask, jev-delegate-mcp, jev-chat, jev-gateway and jev-claude in $dest, and the jev-orchestrate mod in $mods"
case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "note: $dest is not on your PATH; add it to run jev-code from anywhere" ;;
esac
echo "next: cp .jev-router.env.example ~/.jev-router.env && chmod 600 ~/.jev-router.env, add your JEV_API_KEY, then run: jev-code doctor"
