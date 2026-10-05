# jev-router

A small, reliable LLM router for Claude models, in Rust. Jev (TypeSafe System One)
judges each request; code maps the judgment to a model and gates the cheap route on
confidence. It fixes the two things that make routers bad:

1. **Memory across switches.** The model has no memory — the *transcript* does.
   One canonical history (`Transcript`) is sent in full to whichever model a turn
   routes to, so continuity survives every switch. (You lose the prompt *cache*
   when you switch models — that's cost, not memory.)
2. **Guessed thresholds.** The cheap-route confidence cutoff is **calibrated from a
   labeled set**, not guessed. `tune` sweeps it and prints the savings/miss curve.

## Design

- **Judge the task, not the model.** Jev returns a `Choice` over `{simple, standard,
  reasoning, unclear}` with described criteria — never "which model".
- **Code owns policy.** Three tiers: cheap (`claude-haiku-4-5`) when `simple` AND
  `confidence ≥ CHEAP_THRESHOLD`; strong (`claude-opus-5-5`) when `reasoning` AND
  `confidence ≥ STRONG_THRESHOLD`; mid (`claude-sonnet-5-5`) for everything else
  (`standard`, `unclear`, or a low-confidence judgment). Defaults 0.50 / 0.85 come
  from `tune` plus a 1000-prompt run: Haiku answers were checked at the cheap boundary,
  and Sonnet matched Opus on all 52 sampled prompts below the strong one (Opus-graded).
- **Hard rules before the gate** (`router::route_turn`): security-sensitive requests
  (auth, passwords, tokens, crypto, sanitizing, CORS…) always go strong, whatever Jev
  says; a short follow-up Jev isn't confident about ("go on", "why?") stays on the
  model that answered the previous turn. `tune` calibrates Jev's judgment only and
  ignores these rules.
- **Verify variant** (recommended, in `router::answer_is_adequate`): run cheap, then
  ask Jev a `Noul` "does this fully answer it?" and escalate if low — the cutoff then
  tracks answer quality, which is far more stable than predicting difficulty up front.

## Run

```bash
export TYPESAFE_API_KEY=...        # Jev (JEV_API_KEY also works)
# Claude runs through Claude Code (`claude -p`) on your subscription — no API key.

# 1) Calibrate the threshold from your labeled cases (no guessing)
cargo run --bin tune                # reads eval/cases.json; prints the sweep + a suggested cutoff

# 2) Chat with the router at that threshold
cargo run --bin chat               # defaults: CHEAP_THRESHOLD=0.50 STRONG_THRESHOLD=0.85
```

### Code mode

```bash
cd ~/some-project && cargo run --manifest-path ~/Playground/jev-router/Cargo.toml --bin code
# or: cargo run --bin code /path/to/project
```

A routed coding agent: every turn runs Claude Code in the project with tools on,
resuming one Claude Code session so files read, edits, and command output carry
across turns and model switches. Policy differs from chat:

- Stricter cutoffs than chat (`CODE_CHEAP_THRESHOLD=0.60`, `CODE_STRONG_THRESHOLD=0.80`):
  in a 24-answer check on a real monorepo, verified against the code, both routing
  misses sat right at the chat cutoffs.
- Security has two levels: vulnerability/secret/crypto/audit terms always go to Opus;
  auth/session/token plumbing goes to Opus only when the turn changes files, and is
  never sent to Haiku.
- Haiku only for confidently-simple **read-only** turns (Jev also judges "does this
  change files or run commands?"), and only with Read/Grep/Glob tools.
- Edits, tests, and commands get Sonnet at minimum; confident reasoning or
  security-sensitive turns get Opus.
- Upgrades are immediate; downgrades need `DOWNGRADE_AFTER` (2) confident easier
  turns in a row. Prompt caches are per model, so each switch rebuilds the whole
  context cache — staying put is often cheaper than a cheaper model.
- Edits are auto-accepted (`CODE_PERMISSION_MODE=acceptEdits`); commands need your
  Claude Code permission allowlist or `CODE_ALLOWED_TOOLS="Bash(cargo test) Bash(git status)"`.
  Denied commands are reported, never silently skipped.
- `/model haiku|sonnet|opus|auto` pins or unpins the model, `/new` starts a fresh session.

`eval/cases.json` is a list of `{ request, cheap_was_fine }` — the ground truth is
whether the **cheap** model actually handled each request well. 20–30 real examples
beat any guess. Re-run `tune` when prompts, models, or request mix change.

## Files

- `src/anthropic.rs` — Claude client via Claude Code (`claude -p`); sends the full history each call.
- `src/typesafe.rs` — Jev client (`choice`, `noul`).
- `src/router.rs` — the routing judgment + confidence gate + verify-Noul.
- `src/transcript.rs` — the one canonical conversation history.
- `src/bin/chat.rs` — interactive router REPL.
- `src/bin/tune.rs` — threshold calibration (the sweep).
- `src/bin/code.rs` — routed coding agent over Claude Code sessions.
- `src/bin/route.rs` — routing-only dry run over a prompt file (Jev only, no Claude calls).

## Notes

- Keys stay in env / server-side.
- Set `CLAUDE_BIN` if `claude` isn't on your PATH.
- Thresholds aren't permanent; they drift with your inputs — recalibrate periodically.
