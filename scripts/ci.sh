#!/usr/bin/env bash
# One command that runs every gate (docs/dashboard/DESIGN.md 11.4).
#
#   scripts/ci.sh               stages 1-13, fail-fast, then the stage table
#   scripts/ci.sh --keep-going  run every stage even after a failure (still exits non-zero)
#   scripts/ci.sh --release     SKIPPED and NOT RUN count as failures (stage 12 is advisory)
#   scripts/ci.sh --bootstrap   install the missing cargo tools at pinned versions (no root)
#   scripts/ci.sh --only N[,N]  run only these stages
#
# Stage 1 is the engine's own gate and comes first. Stages 2-9 are Flashboard's. Stages
# 10-13 (real-binary smoke test, coverage, fuzz, WebDriver) are T12's: until then they
# report SKIPPED with the reason; no stage is ever skipped silently.
set -u

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 2

# The app crates, always named explicitly: a bare cargo command at the root selects the engine.
P=(-p jev-usage-core -p jev-usage-io -p jev-dashboard)

# Pinned tool versions for --bootstrap.
CARGO_DENY_VERSION="0.22.0"
CARGO_AUDIT_VERSION="0.23.0"

# No gate may start an installer through a mise wrapper (DESIGN 3.1, 8.6).
export MISE_OFFLINE=1 MISE_AUTO_INSTALL=false MISE_EXEC_AUTO_INSTALL=false MISE_NOT_FOUND_AUTO_INSTALL=false

keep_going=0
release=0
only=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep-going) keep_going=1 ;;
    --release) release=1 ;;
    --only) shift; only=",${1:-},";;
    --bootstrap)
      set -e
      command -v cargo-deny >/dev/null 2>&1 || cargo install cargo-deny --version "$CARGO_DENY_VERSION" --locked
      command -v cargo-audit >/dev/null 2>&1 || cargo install cargo-audit --version "$CARGO_AUDIT_VERSION" --locked
      echo "bootstrap: cargo-deny and cargo-audit are installed."
      echo "bootstrap: cargo-llvm-cov, cargo-fuzz, the nightly toolchain and tauri-driver are added by T12 (stages 10-13)."
      exit 0
      ;;
    -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "ci.sh: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

names=()
results=()
notes=()
times=()
failed=0
stopped=0

# record <name> <result> <note> <seconds>
record() {
  names+=("$1"); results+=("$2"); notes+=("$3"); times+=("$4")
  case "$2" in
    FAIL) failed=1 ;;
    SKIPPED|"NOT RUN") if [ "$release" -eq 1 ] && [ "${5:-required}" != "advisory" ]; then failed=1; fi ;;
  esac
}

wanted() {
  [ -z "$only" ] && return 0
  case "$only" in *",$1,"*) return 0 ;; esac
  return 1
}

# stage <number> <name> <function>
stage() {
  local number="$1" name="$2" fn="$3" start end status
  wanted "$number" || return 0
  if [ "$stopped" -eq 1 ]; then
    record "$number $name" "NOT REACHED" "an earlier stage failed (fail-fast)" 0
    return 0
  fi
  printf '\n=== stage %s: %s ===\n' "$number" "$name"
  start=$(date +%s)
  # Not inside `if`: a command tested by `if` runs with errexit off, which would let a
  # failing command in the middle of a stage pass unnoticed.
  ( set -e; "$fn" )
  status=$?
  end=$(date +%s)
  if [ "$status" -eq 0 ]; then
    record "$number $name" "PASS" "" $((end - start))
  else
    record "$number $name" "FAIL" "exit $status" $((end - start))
    [ "$keep_going" -eq 1 ] || stopped=1
  fi
}

# skipped <number> <name> <reason> [advisory]
skipped() {
  wanted "$1" || return 0
  printf '\n=== stage %s: %s ===\nSKIPPED: %s\n' "$1" "$2" "$3"
  record "$1 $2" "SKIPPED" "$3" 0 "${4:-required}"
}

stage_engine() {
  cargo build --all-targets --locked
  cargo test --locked
}

stage_ui_build_and_static() {
  # The UI build comes first: the shell crate embeds dashboard/dist at compile time, so
  # every cargo command that compiles jev-dashboard needs it.
  pnpm --dir dashboard install --frozen-lockfile
  pnpm --dir dashboard build
  cargo fmt --check "${P[@]}"
  cargo clippy "${P[@]}" --all-targets --locked -- --no-deps -D warnings
}

stage_rust_tests() {
  PROPTEST_CASES=4096 cargo test "${P[@]}" --locked
}

stage_bindings() {
  local dir="dashboard/src/lib/ipc/bindings" before
  before="$(mktemp -d)"
  cp -r "$dir/." "$before/"
  cargo test -p jev-usage-core --features bindings --locked export_bindings
  if ! diff -r "$before" "$dir"; then
    rm -rf "$before"
    echo "bindings drift: $dir was out of date; commit the regenerated files" >&2
    return 1
  fi
  rm -rf "$before"
}

stage_supply_chain() {
  command -v cargo-deny >/dev/null 2>&1 || { echo "cargo-deny is not installed: scripts/ci.sh --bootstrap" >&2; return 1; }
  cargo deny --workspace --locked check
  if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit
  else
    echo "cargo-audit is not installed: scripts/ci.sh --bootstrap" >&2
    return 1
  fi
}

stage_ui_static_and_unit() {
  pnpm --dir dashboard lint
  pnpm --dir dashboard check
  pnpm --dir dashboard test
  pnpm --dir dashboard audit --prod
}

# gzip_total <glob>...: the summed gzip size, in bytes, of the files that exist.
gzip_total() {
  local total=0 f size
  for f in "$@"; do
    [ -f "$f" ] || continue
    size=$(gzip -9 -c "$f" | wc -c)
    total=$((total + size))
  done
  echo "$total"
}

raw_total() {
  local total=0 f size
  for f in "$@"; do
    [ -f "$f" ] || continue
    size=$(wc -c < "$f")
    total=$((total + size))
  done
  echo "$total"
}

stage_budgets() {
  local assets="dashboard/dist/assets" js css fonts ok=0
  [ -d "$assets" ] || { echo "no dashboard/dist/assets: stage 2 builds it" >&2; return 1; }
  js=$(gzip_total "$assets"/*.js)
  css=$(gzip_total "$assets"/*.css)
  fonts=$(raw_total "$assets"/*.woff2 "$assets"/*.woff "$assets"/*.ttf)
  printf 'JavaScript %d bytes gzip (budget 71680)\n' "$js"
  printf 'CSS        %d bytes gzip (budget 12288)\n' "$css"
  printf 'fonts      %d bytes      (budget 122880)\n' "$fonts"
  [ "$js" -le 71680 ] || { echo "JavaScript is over budget" >&2; ok=1; }
  [ "$css" -le 12288 ] || { echo "CSS is over budget" >&2; ok=1; }
  [ "$fonts" -le 122880 ] || { echo "fonts are over budget" >&2; ok=1; }
  return "$ok"
}

stage_screens() {
  pnpm --dir dashboard test:screens
}

stage_release_build() {
  pnpm --dir dashboard tauri build --no-bundle -- --profile dist --locked
  [ -x target/dist/jev-dashboard ] || { echo "target/dist/jev-dashboard is missing" >&2; return 1; }
  printf 'binary %d bytes (budget 15728640)\n' "$(wc -c < target/dist/jev-dashboard)"
  [ "$(wc -c < target/dist/jev-dashboard)" -le 15728640 ]
}

total_start=$(date +%s)

stage 1 "engine build and tests" stage_engine
stage 2 "UI build, fmt, clippy (app crates)" stage_ui_build_and_static
stage 3 "app Rust tests (property tests at CI counts)" stage_rust_tests
stage 4 "bindings drift" stage_bindings
stage 5 "cargo deny, cargo audit" stage_supply_chain
stage 6 "UI lint, types, check-types, unit tests, audit" stage_ui_static_and_unit
stage 7 "bundle budgets" stage_budgets
stage 8 "Playwright screens" stage_screens
stage 9 "release build (profile dist)" stage_release_build
skipped 10 "real-binary smoke test" "not built yet: T12 (dashboard/e2e/smoke)"
skipped 11 "coverage thresholds" "not built yet: T12 (cargo-llvm-cov)"
skipped 12 "fuzz" "not built yet: T12 (cargo-fuzz, nightly); advisory" advisory
skipped 13 "WebDriver end-to-end" "not built yet: T12 (tauri-driver + WebKitWebDriver)"

total_end=$(date +%s)

printf '\n%-52s %-12s %6s  %s\n' "stage" "result" "time" "note"
printf '%-52s %-12s %6s  %s\n' "-----" "------" "----" "----"
for i in "${!names[@]}"; do
  printf '%-52s %-12s %5ss  %s\n' "${names[$i]}" "${results[$i]}" "${times[$i]}" "${notes[$i]}"
done
printf '\ntotal %ss' $((total_end - total_start))
if [ "$failed" -eq 1 ]; then
  printf ': FAILED\n'
  exit 1
fi
printf ': ok\n'
