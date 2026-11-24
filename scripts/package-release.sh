#!/usr/bin/env bash
# Package a release build into the archive `/update` downloads, and its checksum.
#
#   scripts/package-release.sh <tag> <platform> [out-dir]
#
# Run it after `JEV_RELEASE_BUILD=1 cargo build --release --locked --bin code --bin ask --bin chat
# --bin gateway --bin delegate_mcp`. <platform> is linux-x86_64, linux-aarch64, macos-x86_64,
# macos-aarch64 or windows-x86_64. It writes <out-dir>/jev-code-<tag>-<platform>.tar.gz (a .zip for
# Windows) and the same name + .sha256. The names, the folder inside and the files must match
# crates/tui/src/app/release_update.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
tag="${1:?usage: package-release.sh <tag> <platform> [out-dir]}"
platform="${2:?usage: package-release.sh <tag> <platform> [out-dir]}"
out="${3:-dist}"
name="jev-code-$tag-$platform"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name/mods" "$out"
out="$(cd "$out" && pwd)"
case "$platform" in
  windows-*)
    for pair in code:jev-code ask:jev-ask delegate_mcp:jev-delegate-mcp chat:jev-chat gateway:jev-gateway; do
      install -m755 "target/release/${pair%%:*}.exe" "$stage/$name/${pair##*:}.exe"
    done
    install -m755 scripts/jev-claude.cmd "$stage/$name/jev-claude.cmd"
    install -m755 scripts/jev-claude.ps1 "$stage/$name/jev-claude.ps1"
    archive="$name.zip"
    ;;
  *)
    install -m755 target/release/code "$stage/$name/jev-code"
    install -m755 target/release/ask "$stage/$name/jev-ask"
    install -m755 target/release/delegate_mcp "$stage/$name/jev-delegate-mcp"
    install -m755 target/release/chat "$stage/$name/jev-chat"
    install -m755 target/release/gateway "$stage/$name/jev-gateway"
    install -m755 scripts/jev-claude "$stage/$name/jev-claude"
    archive="$name.tar.gz"
    ;;
esac
cp -r mods/jev-orchestrate "$stage/$name/mods/jev-orchestrate"
case "$archive" in
  *.zip) (cd "$stage" && 7z a -tzip -bso0 "$out/$archive" "$name") ;;
  *) tar -czf "$out/$archive" -C "$stage" "$name" ;;
esac
(
  cd "$out"
  if command -v sha256sum >/dev/null; then sha256sum "$archive"; else shasum -a 256 "$archive"; fi > "$archive.sha256"
)
echo "wrote $out/$archive and $archive.sha256"
