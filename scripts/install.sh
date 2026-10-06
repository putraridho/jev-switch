#!/usr/bin/env bash
# Build jev-router in release mode and install the code agent as `jev-code`.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="${1:-$HOME/.local/bin}"
cargo build --release --bin code
install -Dm755 target/release/code "$dest/jev-code"
echo "installed $dest/jev-code"
