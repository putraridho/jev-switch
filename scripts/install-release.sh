#!/bin/sh
# Install the newest jev-code release (no Rust needed): download, check the SHA-256, install.
#
#   curl -fsSL https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.sh | sh
#   sh install-release.sh [dest]        dest defaults to ~/.local/bin
#
# Afterwards `jev-code update` (or /update inside jev-code) keeps it current. To build from a
# clone instead, use scripts/install.sh.
set -eu
repo="putraridho/jev-switch"
dest="${1:-$HOME/.local/bin}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x86_64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-aarch64 ;;
  Darwin-x86_64) platform=macos-x86_64 ;;
  Darwin-arm64) platform=macos-aarch64 ;;
  *) echo "no prebuilt jev-code for $(uname -s)-$(uname -m); build it with scripts/install.sh" >&2; exit 1 ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tag="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
case "$tag" in v[0-9]*) ;; *) echo "could not find the newest release of $repo" >&2; exit 1 ;; esac
name="jev-code-$tag-$platform"
base="https://github.com/$repo/releases/download/$tag"
echo "downloading jev-code $tag for $platform"
curl -fsSL --proto '=https' -o "$work/$name.tar.gz" "$base/$name.tar.gz"
curl -fsSL --proto '=https' -o "$work/$name.tar.gz.sha256" "$base/$name.tar.gz.sha256"
want="$(cut -d ' ' -f 1 "$work/$name.tar.gz.sha256")"
if command -v sha256sum >/dev/null 2>&1; then got="$(sha256sum "$work/$name.tar.gz" | cut -d ' ' -f 1)"; else got="$(shasum -a 256 "$work/$name.tar.gz" | cut -d ' ' -f 1)"; fi
[ "$want" = "$got" ] || { echo "checksum mismatch (expected $want, got $got); nothing was installed" >&2; exit 1; }
tar -xzf "$work/$name.tar.gz" -C "$work"
mkdir -p "$dest"
for p in jev-code jev-ask jev-delegate-mcp jev-chat jev-gateway jev-claude; do
  install -m 755 "$work/$name/$p" "$dest/.$p.new"
  mv "$dest/.$p.new" "$dest/$p"
done
mods="${XDG_DATA_HOME:-$HOME/.local/share}/jev-router/mods"
mkdir -p "$mods"
rm -rf "$mods/jev-orchestrate.new" "$mods/jev-orchestrate"
cp -r "$work/$name/mods/jev-orchestrate" "$mods/jev-orchestrate"
pkill -x jev-gateway 2>/dev/null || true
echo "installed jev-code $tag in $dest, and the jev-orchestrate mod in $mods"
case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "note: $dest is not on your PATH; add it to run jev-code from anywhere" ;;
esac
echo "next: add your JEV_API_KEY to ~/.jev-router.env (chmod 600), then run: jev-code doctor"
