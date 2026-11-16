# provider_eval

The routing table (task type to provider chain) is a guess. This replays graded prompts from the
lunagraph training run (`~/.local/state/jev-router/train-lunagraph`) on each provider, grades the
answers blind, and compares quality per task type, so the table can be corrected from data.

**Cost warning:** `run` and `grade` spend real quota (each call is a full agentic turn in the repo
snapshot). Both default to a dry run that prints call counts; nothing is spent without `--yes`.
`plan` and `report` never call a model. Lunagraph categories map to router task types by an
approximation (`TASK_MAP` at the top of the script); frontend, docs and current_info are not covered.

```
cargo build --release --bin ask
python3 -I scripts/provider_eval.py plan   --providers codex:gpt-6.1-sol,gemini:gemini-4-argon --per-type 5 [--cross-grader codex:gpt-6.1-sol] --out DIR
python3 -I scripts/provider_eval.py run    --out DIR --yes      # resumable; stops a provider on quota
python3 -I scripts/provider_eval.py grade  --out DIR --yes      # blind A/B/C grading; resumable
python3 -I scripts/provider_eval.py report --out DIR            # tables + DIR/report.md
```

Add `--cross-grader` so a second model re-grades a sample and the report can flag grader
self-preference (the default grader and the Sonnet baseline are both Claude).

The table lives in `src/catalog.rs` (`default_chain`). To apply results without rebuilding, point
`JEV_CHAINS` at a JSON file such as `{"security": ["codex:gpt-6.1-sol", "claude:claude-opus-5-5"]}`;
the report prints a quality-only suggestion in that format. Add cost before choosing: pick the knee.

Tests: `python3 -I scripts/test_provider_eval.py` (no network, no quota).
