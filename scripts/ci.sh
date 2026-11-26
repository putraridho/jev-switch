#!/usr/bin/env bash
# One command that runs every gate (docs/dashboard/DESIGN.md 11.4).
#
#   scripts/ci.sh               stages 1-13, fail-fast, then the stage table
#   scripts/ci.sh --no-cache    ignore the result cache (target/ci-cache); --release implies it
#   scripts/ci.sh --keep-going  run every stage even after a failure (still exits non-zero)
#   scripts/ci.sh --release     SKIPPED and NOT RUN count as failures (stage 12 is advisory)
#   scripts/ci.sh --bootstrap   install the missing cargo tools at pinned versions (no root)
#   scripts/ci.sh --only N[,N]  run only these stages
#
# Stage 1 is the engine's own gate and comes first. Stages 2-9 are Flashboard's. Stages
# 10-13 are the real-binary smoke test, coverage thresholds, fuzz (advisory) and the
# WebDriver end-to-end test. A stage whose tool is missing prints SKIPPED (or, for the
# WebDriver stage, NOT RUN) with the install command; no stage is ever skipped silently, and
# NOT RUN is never a pass. The idle budgets (D11.3, ten minutes) are measured separately:
#   dashboard/scripts/measure-idle.sh [--floor <empty-scaffold-binary>]
set -u

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 2

# The app crates, always named explicitly: a bare cargo command at the root selects the engine.
P=(-p jev-usage-core -p jev-usage-io -p jev-dashboard)

# Pinned tool versions for --bootstrap.
CARGO_DENY_VERSION="0.22.0"
CARGO_AUDIT_VERSION="0.23.0"
CARGO_LLVM_COV_VERSION="0.6.16"
CARGO_FUZZ_VERSION="0.12.0"
TAURI_DRIVER_VERSION="2.0.1"
# The pinned nightly for the fuzz stage; override with FLASHBOARD_NIGHTLY.
NIGHTLY="${FLASHBOARD_NIGHTLY:-nightly-2026-11-01}"
FUZZ_SECONDS="${FUZZ_SECONDS:-60}"

# No gate may start an installer through a mise wrapper (DESIGN 3.1, 8.6).
export MISE_OFFLINE=1 MISE_AUTO_INSTALL=false MISE_EXEC_AUTO_INSTALL=false MISE_NOT_FOUND_AUTO_INSTALL=false

keep_going=0
release=0
use_cache=1
only=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --keep-going) keep_going=1 ;;
    --no-cache) use_cache=0 ;;
    --release) release=1; use_cache=0 ;;
    --only) shift; only=",${1:-},";;
    --bootstrap)
      # No root. Every cargo tool is installed --locked at a pinned version.
      set -e
      command -v cargo-deny >/dev/null 2>&1 || cargo install cargo-deny --version "$CARGO_DENY_VERSION" --locked
      command -v cargo-audit >/dev/null 2>&1 || cargo install cargo-audit --version "$CARGO_AUDIT_VERSION" --locked
      command -v cargo-llvm-cov >/dev/null 2>&1 || cargo install cargo-llvm-cov --version "$CARGO_LLVM_COV_VERSION" --locked
      rustup component add llvm-tools-preview
      command -v cargo-fuzz >/dev/null 2>&1 || cargo install cargo-fuzz --version "$CARGO_FUZZ_VERSION" --locked
      rustup toolchain install "$NIGHTLY" --profile minimal
      command -v tauri-driver >/dev/null 2>&1 || cargo install tauri-driver --version "$TAURI_DRIVER_VERSION" --locked
      pnpm --dir dashboard exec playwright install chromium
      echo "bootstrap: done. WebKitWebDriver needs root: sudo pacman -S webkitgtk-6.0"
      exit 0
      ;;
    -h|--help) sed -n '2,16p' "${BASH_SOURCE[0]}"; exit 0 ;;
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

# Result cache: a stage that passed on exactly the same inputs is not run again. The key is a
# hash of the tracked and untracked (not ignored) files under the stage's input paths plus
# ci.sh itself. Stages that depend on the clock or the network (supply chain, 10-13) are never
# cached. `--no-cache` and `--release` always run everything.
cache_dir="$root/target/ci-cache"
declare -A INPUTS=(
  [1]="Cargo.toml Cargo.lock src crates tests"
  [2]="dashboard Cargo.toml Cargo.lock"
  [3]="dashboard Cargo.toml Cargo.lock"
  [4]="dashboard/crates dashboard/src/lib/ipc/bindings Cargo.lock"
  [6]="dashboard"
  [7]="dashboard"
  [8]="dashboard"
  [9]="dashboard Cargo.toml Cargo.lock"
)

inputs_hash() {
  # shellcheck disable=SC2086
  { git ls-files -co --exclude-standard -- $1 scripts/ci.sh | LC_ALL=C sort -u | while IFS= read -r f; do
      [ -f "$f" ] && sha1sum -- "$f"
    done; } | sha1sum | cut -d' ' -f1
}

# run_one <number> <function> <out-prefix>: runs the stage (or reuses its cached pass) and
# writes <out-prefix>.status (PASS, CACHED or FAIL), <out-prefix>.time and <out-prefix>.log.
run_one() {
  local number="$1" fn="$2" out="$3" key="" start end status
  start=$(date +%s)
  if [ "$use_cache" -eq 1 ] && [ -n "${INPUTS[$number]:-}" ]; then
    key="$(inputs_hash "${INPUTS[$number]}")"
    if [ -f "$cache_dir/$number" ] && [ "$(cat "$cache_dir/$number")" = "$key" ] \
       && { [ "$number" != 2 ] || [ -d dashboard/dist ]; }; then
      echo "cached: inputs unchanged since the last pass" > "$out.log"
      echo CACHED > "$out.status"; echo 0 > "$out.time"
      return 0
    fi
  fi
  # Not inside `if`: a command tested by `if` runs with errexit off, which would let a
  # failing command in the middle of a stage pass unnoticed.
  ( set -e; "$fn" ) > "$out.log" 2>&1
  status=$?
  end=$(date +%s)
  echo $((end - start)) > "$out.time"
  if [ "$status" -eq 0 ]; then
    [ -z "$key" ] || { mkdir -p "$cache_dir"; echo "$key" > "$cache_dir/$number"; }
    echo PASS > "$out.status"
  else
    [ -z "$key" ] || rm -f "$cache_dir/$number"
    echo "FAIL exit $status" > "$out.status"
  fi
}

# collect <number> <name> <out-prefix>: print the log and record the result.
collect() {
  local number="$1" name="$2" out="$3" status secs
  status="$(cat "$out.status")"; secs="$(cat "$out.time")"
  printf '\n=== stage %s: %s ===\n' "$number" "$name"
  cat "$out.log"
  case "$status" in
    PASS) record "$number $name" "PASS" "" "$secs" ;;
    CACHED) record "$number $name" "PASS" "cached" 0 ;;
    *) record "$number $name" "FAIL" "${status#FAIL }" "$secs"
       [ "$keep_going" -eq 1 ] || stopped=1 ;;
  esac
}

tmp_out="$(mktemp -d)"
trap 'rm -rf "$tmp_out"' EXIT

# stage <number> <name> <function>: serial.
stage() {
  local number="$1" name="$2" fn="$3"
  wanted "$number" || return 0
  if [ "$stopped" -eq 1 ]; then
    record "$number $name" "NOT REACHED" "an earlier stage failed (fail-fast)" 0
    return 0
  fi
  run_one "$number" "$fn" "$tmp_out/$number"
  collect "$number" "$name" "$tmp_out/$number"
}

# pstage <number> <name> <function>: start in the background; pwait collects in order.
pnames=(); pnums=()
pstage() {
  local number="$1" name="$2" fn="$3"
  wanted "$number" || return 0
  if [ "$stopped" -eq 1 ]; then
    record "$number $name" "NOT REACHED" "an earlier stage failed (fail-fast)" 0
    return 0
  fi
  pnums+=("$number"); pnames+=("$name")
  run_one "$number" "$fn" "$tmp_out/$number" &
}
pwait() {
  wait
  local i
  for i in "${!pnums[@]}"; do collect "${pnums[$i]}" "${pnames[$i]}" "$tmp_out/${pnums[$i]}"; done
  pnums=(); pnames=()
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

# run_external <number> <name> <advisory|required> <command...>: a stage whose script reports its
# own state: 0 pass, 3 NOT RUN (a tool or the display is missing), 4 pass with a noted caveat,
# anything else FAIL.
run_external() {
  local number="$1" name="$2" kind="$3" start end status
  shift 3
  wanted "$number" || return 0
  if [ "$stopped" -eq 1 ]; then
    record "$number $name" "NOT REACHED" "an earlier stage failed (fail-fast)" 0
    return 0
  fi
  printf '\n=== stage %s: %s ===\n' "$number" "$name"
  start=$(date +%s)
  "$@"
  status=$?
  end=$(date +%s)
  case "$status" in
    0) record "$number $name" "PASS" "" $((end - start)) ;;
    3) record "$number $name" "NOT RUN" "see the message above for the install command" $((end - start)) "$kind" ;;
    4) record "$number $name" "PASS" "with a caveat: see the message above" $((end - start)) ;;
    *) record "$number $name" "FAIL" "exit $status" $((end - start))
       [ "$keep_going" -eq 1 ] || stopped=1 ;;
  esac
}

stage_smoke() {
  dashboard/e2e/smoke/smoke.sh target/dist/jev-dashboard
}

stage_webdriver() {
  python3 -I dashboard/e2e/webdriver/webdriver.py target/dist/jev-dashboard
}

stage_coverage() {
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "SKIPPED: cargo-llvm-cov is not installed: scripts/ci.sh --bootstrap" >&2
    return 3
  fi
  # Line coverage: core >= 95 %, io >= 85 % (D11.3); the shell is glue and is covered by e2e.
  # The engine's own crates (root crates/ and src/) are a dependency of usage-io and have their
  # own gate in stage 1; only Flashboard's files count here.
  local ignore="(/tests/|/stubs/|/fuzz/|/src-tauri/|^$root/(crates|src)/)"
  cargo llvm-cov -p jev-usage-core --locked --ignore-filename-regex "$ignore" --fail-under-lines 95 || return 1
  cargo llvm-cov -p jev-usage-io --locked --ignore-filename-regex "$ignore|/usage-core/" --fail-under-lines 85 || return 1
  # Vitest: src/lib logic >= 95 %, components >= 85 % (thresholds live in vitest.config.ts).
  pnpm --dir dashboard exec vitest run --coverage
}

stage_fuzz() {
  if ! command -v cargo-fuzz >/dev/null 2>&1 || ! rustup toolchain list | grep -q "^${NIGHTLY}"; then
    echo "SKIPPED: cargo-fuzz or the $NIGHTLY toolchain is missing: scripts/ci.sh --bootstrap" >&2
    return 3
  fi
  local target seeds corpus
  for seeds in dashboard/fuzz/seeds/*/; do
    target="$(basename "$seeds")"
    corpus="dashboard/fuzz/corpus/$target"
    mkdir -p "$corpus"
    cp -n "$seeds"* "$corpus"/ 2>/dev/null || true
    cargo +"$NIGHTLY" fuzz run --fuzz-dir dashboard/fuzz "$target" -- -max_total_time="$FUZZ_SECONDS" -rss_limit_mb=2048 || return 1
  done
}

# A stage that returns 3 from a plain function: SKIPPED, not a failure.
soft_stage() {
  local number="$1" name="$2" fn="$3" kind="$4" start end status
  wanted "$number" || return 0
  if [ "$stopped" -eq 1 ]; then
    record "$number $name" "NOT REACHED" "an earlier stage failed (fail-fast)" 0
    return 0
  fi
  printf '\n=== stage %s: %s ===\n' "$number" "$name"
  start=$(date +%s)
  ( set -e; "$fn" )
  status=$?
  end=$(date +%s)
  case "$status" in
    0) record "$number $name" "PASS" "" $((end - start)) ;;
    3) record "$number $name" "SKIPPED" "tool missing: scripts/ci.sh --bootstrap" $((end - start)) "$kind" ;;
    *) record "$number $name" "FAIL" "exit $status" $((end - start))
       [ "$keep_going" -eq 1 ] || stopped=1 ;;
  esac
}

total_start=$(date +%s)

stage 1 "engine build and tests" stage_engine
stage 2 "UI build, fmt, clippy (app crates)" stage_ui_build_and_static
# Stages 3-8 only read what stage 2 built, so they run side by side (cargo serialises its own
# build lock; the pnpm, Playwright and audit stages overlap with it).
pstage 3 "app Rust tests (property tests at CI counts)" stage_rust_tests
pstage 4 "bindings drift" stage_bindings
pstage 5 "cargo deny, cargo audit" stage_supply_chain
pstage 6 "UI lint, types, check-types, unit tests, audit" stage_ui_static_and_unit
pstage 7 "bundle budgets" stage_budgets
pstage 8 "Playwright screens" stage_screens
pwait
stage 9 "release build (profile dist)" stage_release_build
run_external 10 "real-binary smoke test" required stage_smoke
soft_stage 11 "coverage thresholds" stage_coverage required
soft_stage 12 "fuzz (advisory)" stage_fuzz advisory
run_external 13 "WebDriver end-to-end" required stage_webdriver

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
