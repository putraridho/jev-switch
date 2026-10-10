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
jev-chat                           # after scripts/install.sh; or: cargo run --bin chat
```

### Claude Code interface (`jev-claude`)

```bash
./scripts/install.sh
cd ~/some-project && jev-claude     # normal Claude Code, with "Jev Code" selected in /model
```

`jev-claude` starts `jev-gateway` (a local Anthropic-format gateway on
127.0.0.1:18780) if needed, then runs `claude` with `ANTHROPIC_BASE_URL` pointing at
it and a custom `/model` entry, **Jev Code** (`ANTHROPIC_CUSTOM_MODEL_OPTION`). Your
claude.ai / Max login passes through unchanged; plain `claude` is unaffected.

- The gateway forwards every request to Anthropic byte-for-byte and streams the
  response back. It changes only the `model` field, and only when it is `jev-code`.
- It decides once per user prompt (Claude Code's `x-claude-code-prompt-id` hint
  header, enabled with `CLAUDE_CODE_GATEWAY_HINT_HEADERS=1`), so all API calls in a
  turn use one model, with code mode's policy: Sonnet by default; Opus for confident
  reasoning, security work (keywords or Jev's `security_review`), or edits to auth
  code; slow downgrades.
  Background requests (titles, summaries) go to Haiku; subagents and compaction stay
  on the turn's model.
- Picking Sonnet or Opus in `/model` bypasses routing for that session.
- Decisions are logged to `~/.local/state/jev-router/gateway.jsonl` (gateway output
  in `gateway.out`). Stop it with `pkill jev-gateway`; `jev-claude` restarts it.
- Claude Code prints an "unrecognized model" note for `jev-code`; it is harmless.
  The `[1m]` suffix tells Claude Code the context window is 1M, like Sonnet and Opus.

### Code mode (`jev-code`)

```bash
./scripts/install.sh          # builds release, installs ~/.local/bin/jev-code and jev-chat
cd ~/some-project && jev-code # or: jev-code /path/to/project; jev-code --help
```

The Jev key is read from `TYPESAFE_API_KEY` / `JEV_API_KEY`, or from
`~/.jev-router.env` (`JEV_API_KEY=...`), so it works from any directory. Re-run
`scripts/install.sh` after changing the code.

A routed coding agent: every turn runs Claude Code in the project with tools on,
resuming one Claude Code session so files read, edits, and command output carry
across turns and model switches. Two tiers, no Haiku:

- Sonnet by default; Opus for confident reasoning (`CODE_STRONG_THRESHOLD=0.85`),
  strong security terms (vulnerabilities, secrets, crypto, audits), security
  questions without those terms, and edits to auth/session code.
- Security questions without keywords ("can a viewer see who the org shares files
  with?", "who can call this?") are caught by a third Jev judgment,
  `security_review`, at `CODE_SECURITY_P` (0.85).
- Both cutoffs come from lunagraph: 2,296 prompts over the whole monorepo, routed
  with every Jev signal recorded (`ROUTE_SIGNALS=1`), 408 Sonnet answers graded
  against the code by Opus, and Opus reruns of every Sonnet miss, graded the same
  way. Sonnet was correct on 383 (94%). Replaying candidate policies on the
  recorded signals:

  | policy | turns on Opus | Sonnet misses Opus fixes, left on Sonnet |
  |---|---|---|
  | reasoning 0.75, security 0.90 (before) | 31% | 7 |
  | reasoning 0.85, security 0.85 (now) | 28% | 5 |
  | reasoning 0.90, security 0.85 | 26% | 6 |
  | security only, 0.85 (no reasoning rule) | 15% | 7 |

  Sonnet's answers at reasoning confidence 0.75–0.85 were 31 correct and 2 partial
  that Opus also got partly wrong. The keyword-less security questions it missed and
  Opus fixed scored 0.86–0.97. The 5 left are misses no signal predicts: a refactor
  and a small edit Jev called simple, two security-sounding questions that weren't
  (a theme-token lookup, md5 for content hashes), and one security question Jev scored
  0.60. Strong keywords still go to Opus unconditionally: gating them on
  `security_review` would save under 2% and they also cover Jev being down.
- No Haiku. Tested on five repos (2,500 questions, 120 answers verified against the
  code): Haiku's code explanations held up ~85% at every Jev confidence, so no
  cutoff separated its misses, and prompt caches are per model, so its cache-cold
  turns saved little. Sonnet held up on 60 of 68 graded answers.
- Upgrades are immediate; stepping back down to Sonnet needs `DOWNGRADE_AFTER` (2)
  confident easier turns, because each switch rebuilds the context cache. Short
  replies Jev calls `unclear` count as easier when `edit p` < 0.2, and an uncertain
  turn holds the streak instead of resetting it; only a turn that needs the current
  tier resets it.
- Edits are auto-accepted (`CODE_PERMISSION_MODE=acceptEdits`); commands need your
  Claude Code permission allowlist or `CODE_ALLOWED_TOOLS="Bash(cargo test) Bash(git status)"`.
  Denied commands are reported, never silently skipped.
- `/model sonnet|opus|auto` pins or unpins the model, `/new` starts a fresh session.

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
- `src/bin/gateway.rs` — local gateway that adds the Jev Code model to Claude Code.
- `scripts/jev-claude` — launcher: starts the gateway and opens Claude Code on Jev Code.
- `src/bin/code.rs` — routed coding agent over Claude Code sessions.
- `src/bin/route.rs` — routing-only dry run over a prompt file (Jev only, no Claude calls);
  `ROUTE_MODE=code ROUTE_SIGNALS=1` also records every Jev judgment per prompt for
  replaying policies offline.

## Notes

- Keys stay in env / server-side.
- Set `CLAUDE_BIN` if `claude` isn't on your PATH.
- Thresholds aren't permanent; they drift with your inputs — recalibrate periodically.
