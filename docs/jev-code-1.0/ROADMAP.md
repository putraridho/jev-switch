# jev-code 1.0.0 roadmap

jev-code 1.0.0 is the mega update from 0.2.0. The full pillar descriptions (31 pillars) live
in the planning file; this page is the tracked order of work. Tiers: **A** = must ship in 1.0,
**B** = should ship in 1.0, **C** = 1.x.

## Baseline (2026-11-18, master c3cdb63)

- `cargo build --all-targets`: warning-free. `cargo test`: 953 pass in the main suite.
- `python3 -I scripts/test_pty_ux.py`: 45/45 scenarios pass.
- Piped start-up (`printf '/quit\n' | target/release/code`): ~0.16 s warm, ~0.84 s cold.
  Not the interactive prompt time; M0 adds a real measurement.

## Tiers

| Tier | Pillars |
| ---- | ------- |
| A | 1 Workstation UI, 2 Agent power, 3 Routing intelligence, 15 Speed, 16 Code diff, 18 Cache, 19 Provider visuals, 30 Subagent visuals, 31 Orchestration visuals, 27 Diagnostics, 26 Terminal robustness, 7 Trust and reliability |
| B | 4 Extensibility, 5 Headless and remote, 8 Code intelligence, 9 Context and memory, 11 Providers, 17 Gimmicks (savings ticker, fuel gauge, notifications, palette), 20 Session library, 21 Config and profiles, 22 Privacy policy, 23 Reasoning controls |
| C | 6 Git and review extras, 10 Multimodal, 12 Automation, 13 Evals, 14 Editors and platforms, 24 Recipes, 25 Team, 28 Marketplace, 29 Learning, rest of 17 |

Pillars 30 and 31 share one agents panel and one savings meter; build them together.

## Milestones

| Milestone | Version | Content | Exit check |
| --------- | ------- | ------- | ---------- |
| M0 Baseline | 0.2.x | Speed measurement (`--profile`), perf budget test, this roadmap | numbers recorded here |
| M1 Stabilize | 0.3 | Clean code-review round, live Antigravity turn, 0.2.0 session migration notes | review finds nothing |
| M2 Diff and cache | 0.4 | Pillar 16 (preview before apply, line numbers, word diff, `/review`, checkpoints, `/rewind`), pillar 18 (`/cache`, switch-cost estimate, cache-aware routing) | PTY scenarios per feature |
| M3 Visual identity | 0.5 | Pillar 19 provider themes and attribution; pillar 1 first half (themes, narrow layouts, no-colour) | golden snapshots per provider and colour depth |
| M4 Agents | 0.6 | Pillars 30 and 31 (agents panel, nested transcript, decision card, plan view, savings meter), pillar 2 | fake-worker tests, PTY |
| M5 Workstation | 0.7 | Pillar 1 second half (side panel, multi-line input, paste, history search, collapsible output), pillar 26 | PTY, tmux/ssh checks |
| M6 Speed and trust | 0.8 | Pillar 15 budgets enforced in CI, pillars 7 and 27 | perf test green |
| M7 B-tier | 0.9 | Pick from tier B by value | per-feature tests |
| M8 Release | 1.0.0 | Docs, README, migration notes, install, `doctor` | all tests, clean review round |

DDD refactor (docs/architecture/DDD.md) is optional and only after M5 if wanted.

## Rules for every milestone

- Build warning-free; `cargo test`, PTY suite and Python tests green before merge.
- Every new feature gets a PTY scenario or a golden-render test; use fakes, never LLM quota.
- Strict typing and no secret leaks per `NEXT.md` section 4.
- Commit, merge and install only when the user asks.
