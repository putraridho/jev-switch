#!/usr/bin/env bash
# Run tests, skipping a run whose inputs have not changed since it last passed.
#
#   scripts/cached-test.sh                 every cargo test (lib, bins, tests/)
#   scripts/cached-test.sh changed         only the lib test modules of the files changed
#                                          since HEAD (a quick check, not the gate)
#   scripts/cached-test.sh pty [names...]  the PTY scenarios (all, or those matching names)
#   scripts/cached-test.sh clear           forget every cached pass
#
# A pass is remembered under target/test-cache, keyed by a hash of what the run depends on:
# the sources, manifests and cargo config (and for PTY runs the built binary and the script).
# Any change to them, committed or not, means the run happens again. Failures are never cached.
# JEV_TEST_CACHE=off runs everything regardless.
set -euo pipefail
cd "$(dirname "$0")/.."
cache=target/test-cache
mkdir -p "$cache"
mode="${1:-all}"
[ $# -gt 0 ] && shift

hash_files() { # stdin: file names; stdout: one hash of their names and contents
  while IFS= read -r f; do [ -f "$f" ] && { printf '%s\0' "$f"; cat "$f"; }; done | shasum -a 256 | cut -c1-16
}

sources_hash() { # tracked and untracked (not ignored) inputs of the Rust build
  git ls-files -co --exclude-standard -- src tests Cargo.toml Cargo.lock .cargo | LC_ALL=C sort | hash_files
}

cached_run() { # key description command...
  local key="$1" what="$2"; shift 2
  if [ "${JEV_TEST_CACHE:-on}" != off ] && [ -f "$cache/$key" ]; then
    echo "cached: $what passed with these exact inputs ($(cat "$cache/$key")); skipped"
    return 0
  fi
  "$@"
  date '+%Y-%m-%d %H:%M' > "$cache/$key"
}

case "$mode" in
  all)
    cached_run "all-$(sources_hash)" "cargo test" cargo test -q
    ;;
  changed)
    # src/a/b.rs -> a::b, src/a/mod.rs -> a; a manifest, lib.rs or main module change -> all.
    files=$( { git diff --name-only HEAD; git ls-files -o --exclude-standard; } | grep -E '^(src/.*\.rs|Cargo\.(toml|lock)|\.cargo/)' | sort -u || true)
    if [ -z "$files" ]; then echo "nothing changed since HEAD"; exit 0; fi
    filters=()
    while IFS= read -r f; do
      case "$f" in
        src/lib.rs|Cargo.*|.cargo/*) filters=(); break ;;
        src/bin/*) continue ;;
      esac
      m="${f#src/}"; m="${m%.rs}"; m="${m%/mod}"; filters+=("${m//\//::}::")
    done <<< "$files"
    if [ ${#filters[@]} -eq 0 ]; then
      cached_run "all-$(sources_hash)" "cargo test" cargo test -q
    else
      key="changed-$(sources_hash)-$(printf '%s\n' "${filters[@]}" | shasum -a 256 | cut -c1-8)"
      echo "testing: ${filters[*]}"
      # The test harness takes several filters and runs every test matching any of them.
      cached_run "$key" "the changed modules" cargo test -q --lib -- "${filters[@]}"
    fi
    ;;
  pty)
    cargo build -q --bin code
    key="pty-$( { printf '%s\n' target/debug/code scripts/test_pty_ux.py; } | hash_files)-$(printf '%s ' "$@" | shasum -a 256 | cut -c1-8)"
    cached_run "$key" "the PTY scenarios${*:+ matching $*}" python3 -I scripts/test_pty_ux.py "$@"
    ;;
  clear)
    rm -rf "$cache" && echo "test cache cleared"
    ;;
  *)
    sed -n '2,13p' "$0"; exit 2
    ;;
esac
