#!/usr/bin/env bash
# Build jev-router in release mode and install `jev-code` and `jev-chat`.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="${1:-$HOME/.local/bin}"
cargo build --release --bin code --bin chat
install -Dm755 target/release/code "$dest/jev-code"
install -Dm755 target/release/chat "$dest/jev-chat"
echo "installed $dest/jev-code and $dest/jev-chat"
