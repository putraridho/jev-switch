#!/usr/bin/env bash
set -eu

fast=0
if [ "$#" -gt 0 ]; then
    if [ "$#" -eq 1 ] && [ "$1" = "--fast" ]; then
        fast=1
    else
        echo "Usage: scripts/check.sh [--fast]" >&2
        exit 2
    fi
fi

echo '==> Preparing hermetic environment'
unset JEV_API_KEY TYPESAFE_API_KEY DEEPSEEK_API_KEY GEMINI_API_KEY GOOGLE_API_KEY
unset GOOGLE_GENAI_USE_VERTEXAI GOOGLE_GENAI_USE_GCA CARGO_TARGET_DIR
for name in ${!ANTHROPIC_@}; do
    unset "$name"
done
JEV_ROUTER_ENV=$(mktemp)
trap 'rm -f "$JEV_ROUTER_ENV"' EXIT
export JEV_ROUTER_ENV
# No test may ask a real `agy` for its usage.
export JEV_GEMINI_USAGE=off
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

echo '==> Clippy'
if [ "${JEV_CHECK_CLIPPY:-}" = warn ]; then
    cargo clippy --all-targets
else
    cargo clippy --all-targets -- -D warnings
fi

echo '==> Rust tests'
cargo test

if [ "$fast" -eq 1 ]; then
    echo 'Gate passed (fast).'
    exit 0
fi

echo '==> Terminal UX tests'
cargo build --bin code
# The gate never trusts the pass cache: it does not see changes to the suite's own helpers.
JEV_PTY_FRESH=1 python3 -I scripts/test_pty_ux.py

echo '==> Orchestration report tests'
python3 -I scripts/test_orch_report.py
echo '==> Task type accuracy tests'
python3 -I scripts/test_task_type_accuracy.py
echo '==> Provider evaluation tests'
python3 -I scripts/test_provider_eval.py

if [ -f scripts/budget.py ]; then
    echo '==> Budget report'
    python3 -I scripts/budget.py
fi

if [ -d tests/architecture_baseline ]; then
    echo '==> Architecture baseline ratchet'
    python3 -I - <<'PY'
from collections import defaultdict
from pathlib import Path
import subprocess
import sys

directory = Path("tests/architecture_baseline")
parent_exists = subprocess.run(
    ["git", "rev-parse", "--verify", "HEAD~1"], capture_output=True, text=True,
)
if parent_exists.returncode != 0:
    print("Skipping architecture baseline ratchet: no parent.")
    sys.exit(0)

parent = subprocess.check_output(
    ["git", "ls-tree", "--name-only", "HEAD~1", "--", str(directory) + "/"],
    text=True,
)
if not parent.strip():
    print("Skipping architecture baseline ratchet: no parent baseline.")
    sys.exit(0)

parent_files = [name for name in parent.splitlines() if name.endswith(".txt")]
if not parent_files:
    print("Skipping architecture baseline ratchet: no parent baseline.")
    sys.exit(0)

def totals(contents, result):
    for line in contents.splitlines():
        if not line.strip():
            continue
        fields = [field.strip() for field in line.split("|")]
        if len(fields) != 4 or not fields[0]:
            raise ValueError(f"Invalid baseline line: {line}")
        result[fields[0]] += int(fields[3])

previous = defaultdict(int)
current = defaultdict(int)
for name in parent_files:
    contents = subprocess.check_output(["git", "show", f"HEAD~1:{name}"], text=True)
    totals(contents, previous)
for path in directory.glob("*.txt"):
    totals(path.read_text(), current)

increased = False
for check in sorted(current):
    if check not in previous:
        print(f"{check}: a new check, {current[check]} in its first baseline")
        continue
    if current[check] > previous[check]:
        print(f"{check}: {previous[check]} -> {current[check]} (increased)", file=sys.stderr)
        increased = True
if increased:
    sys.exit(1)
print("Architecture baseline totals did not increase.")
PY
fi

echo 'Gate passed (full).'
