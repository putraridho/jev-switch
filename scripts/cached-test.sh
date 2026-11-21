#!/usr/bin/env bash
# Run tests, skipping a run whose inputs have not changed since it last passed.
#
#   scripts/cached-test.sh                 every cargo test (lib, bins, tests/)
#   scripts/cached-test.sh changed         only the lib test modules of the files changed
#                                          since HEAD, crate by crate (a quick check, not the gate)
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
  git ls-files -co --exclude-standard -- src crates tests Cargo.toml Cargo.lock .cargo | LC_ALL=C sort | hash_files
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
    # crates/<ctx>/src/a/b.rs -> `-p jev-<ctx>` filtered to a::b (mod.rs -> its folder); a crate's
    # lib.rs or Cargo.toml -> that whole crate; the test kit, the root manifest, the lock or the
    # cargo config -> everything. Binaries (src/bin) have no tests of their own.
    files=$( { git diff --name-only HEAD; git ls-files -o --exclude-standard; } | grep -E '^(crates/[^/]+/(src/.*\.rs|Cargo\.toml)|Cargo\.(toml|lock)|\.cargo/)' | sort -u || true)
    if [ -z "$files" ]; then echo "nothing changed since HEAD"; exit 0; fi
    plan="" # one "<package> <filter>" line per changed file; "-" as the filter means the whole crate
    everything=0
    while IFS= read -r f; do
      case "$f" in
        Cargo.*|.cargo/*|crates/testkit/*) everything=1; break ;;
      esac
      rest="${f#crates/}"; ctx="${rest%%/*}"
      case "$rest" in
        "$ctx/Cargo.toml"|"$ctx/src/lib.rs") plan="${plan}jev-$ctx -"$'\n' ;;
        *) m="${rest#"$ctx"/src/}"; m="${m%.rs}"; m="${m%/mod}"; plan="${plan}jev-$ctx ${m//\//::}::"$'\n' ;;
      esac
    done <<< "$files"
    if [ "$everything" = 1 ]; then
      cached_run "all-$(sources_hash)" "cargo test" cargo test -q
      exit 0
    fi
    for pkg in $(printf '%s' "$plan" | awk 'NF {print $1}' | sort -u); do
      filters=()
      if ! printf '%s' "$plan" | grep -qx "$pkg -"; then
        while IFS= read -r filter; do filters+=("$filter"); done < <(printf '%s' "$plan" | awk -v p="$pkg" '$1 == p {print $2}' | sort -u)
      fi
      key="changed-$(sources_hash)-$pkg-$(printf '%s\n' ${filters[@]+"${filters[@]}"} | shasum -a 256 | cut -c1-8)"
      echo "testing $pkg: ${filters[*]:-every test}"
      # The test harness takes several filters and runs every test matching any of them.
      cached_run "$key" "$pkg" cargo test -q -p "$pkg" --lib -- ${filters[@]+"${filters[@]}"}
    done
    ;;
  pty)
    cargo build -q --bin code
    key="pty-$( { printf '%s\n' target/debug/code scripts/test_pty_ux.py scripts/pty_ux/*.py; } | hash_files)-$(printf '%s ' "$@" | shasum -a 256 | cut -c1-8)"
    cached_run "$key" "the PTY scenarios${*:+ matching $*}" python3 -I scripts/test_pty_ux.py "$@"
    ;;
  clear)
    rm -rf "$cache" && echo "test cache cleared"
    ;;
  *)
    sed -n '2,13p' "$0"; exit 2
    ;;
esac
