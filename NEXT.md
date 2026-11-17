# Next actions (handoff, 2026-11-17)

Where the work stands and what to do next. Written at the end of a long Claude Code
session so the work can continue from `jev-code`. Read this whole file first.

## 1. State right now

| Stream | Branch / place | State |
|---|---|---|
| jev-code fixes (approvals, `/permissions`, mise-safe probes, orchestration veto, status line, racy-git snapshots, `uifmt/` split) | **merged to `master` (98df79a) and installed** in `~/.local/bin` | 798 Rust tests, 38/38 PTY scenarios, Python tests green. Code review rounds 1–3 each found ~9–10 issues, all fixed. **No clean review round yet.** |
| DDD refactor plan | `docs/architecture/DDD.md` (on `master`) | Plan only, no code moved. Review rounds: 24, then 27 issues (all fixed). **Not converged.** Refactor NOT started. |
| Flashboard (usage dashboard, Tauri 2 + Svelte 5) | branch `dashboard` (has master merged in); `docs/dashboard/DESIGN.md`, `docs/dashboard/TASKS.md` | Design **final** (6 review rounds, capped by the user). **Build NOT started**: wait for the user's go. |

Branches: `master` = installed code. `fix-orch-trigger` = same as master (can be deleted).
`dashboard` = master + Flashboard design docs and fixtures.

Providers on this machine: claude ● (Max), codex ● (paid plan now), gemini/Antigravity ●
(`agy` 1.5.10, Google AI Pro; `gemini-4-carbon` is not on AI Pro, Argon is), deepseek ○ (no
key), grok ✕ (not installed). Antigravity print mode was verified live once (`agy
--output-format stream-json --mode plan --model gemini-4-argon -p ...` returns
`{"event":"result","result":{"status":"SUCCESS","response":...}}`), matching the adapter.
A full `jev-code` turn routed to Antigravity has NOT been run yet.

## 2. Decisions waiting for the user

1. **jev-code**: run code review round 4 on the latest fixes (`/code-review high` on
   `86cd09e..master`), or stop here?
2. **DDD plan**: how many more review rounds (it is not converging yet), then start the
   refactor? Decided already: one crate per context (Cargo workspace, 19 crates under
   `crates/`), untyped JSON only in 3 listed wire files, GitHub Actions CI. Defaults taken:
   U2 typing phase now, U5 no reformat during moves, U6 renames, U7 400-line file / 60-line
   function budgets, U8 merge open branches and freeze first, U9 keep package name
   `jev-router`, U10 pin Rust 1.97.0. See DDD.md section 11.
3. **Flashboard**: start the build (task T00 first)?
4. May we run `agy -p /usage --output-format json` once, to see if Antigravity reports plan
   usage (probably no model quota, unconfirmed)? If yes, Flashboard can show real numbers.
5. Phone access (idea, not planned): agents stay on this PC; a phone app reaches `jev`
   over Tailscale (T3 Code style). Needs: which phone (Android builds here; iPhone needs a
   Mac), and whether to add a VPS fallback later.
6. Optional small feature: attach the `jev` delegate MCP server in `jev-claude` so Claude
   Code can hand sub-tasks to Codex/Antigravity workers on the user's subscriptions.

## 3. Next actions, in order (once the user decides)

### A. jev-code (if round 4 is wanted)
1. `/code-review high 86cd09e..master` (the round-3 fix commits).
2. Verify each finding against the code; fix real ones in a worktree with a test that fails
   before the fix.
3. Verify yourself (section 5), merge to `master`, `bash scripts/install.sh`.
4. Repeat until a round finds nothing.
5. Then: one live `jev-code` turn routed to Antigravity (pin or a task Jev sends there),
   fix what breaks.

### B. DDD refactor
1. More plan review rounds as the user decides (fresh reviewer each round, Fable 5.2,
   fixes go into `docs/architecture/DDD.md` only).
2. Before moving any code: merge open branches (`dashboard` docs are fine to keep apart),
   tag `ddd-start`, freeze feature work (U8).
3. Follow DDD.md section 8 step by step (P0 safety net, P1a untangle in place, P1b
   leaf-first crate extraction, P2–P5). Every step: build warning-free, all tests, PTY
   suite, Python tests green; `git mv` to keep history; binary names, env vars, on-disk and
   log formats byte-identical.

### C. Flashboard
1. Path A or B depends on whether the DDD refactor has started (DESIGN.md D5): Path A if
   not (T00 adds `src/embed.rs`), Path B if it has (depend on `crates/embed` /
   `jev-embed`).
2. T00 (scaffold, one worker, strongest model) per TASKS.md, then Fable 5.2 reviews it,
   then waves 1–4 in parallel worktrees, each task reviewed. WebKitWebDriver e2e is
   required; the Orca screen-reader pass is done together with the user.

## 4. Standing rules (the user's, keep them)

- Validate, review and test yourself, and loop until a round finds nothing. Never trust a
  worker's report without rerunning the tests. (Exception: the Flashboard design loop was
  capped at 6 rounds by the user.)
- Strictest typing: TS has no `any`/`unknown`/`never`/`object`/`as`/`!`/ts-ignore; Rust
  has no `serde_json::Value` inside the program (except the 3 wire files), no stringly
  states, no `unwrap`/`expect` outside tests.
- Readability first, DDD, SOLID, DRY.
- Never leak secrets: keys only in `~/.jev-router.env` (mode 600). Never print, log, store
  or send the account e-mail. Gateway logs (`~/.local/state/jev-router/*.jsonl`) contain
  user prompts: read only allow-listed numeric fields, never print prompts.
- Never run `mise`, a mise wrapper/shim, or `/usr/share/omarchy/bin/*`: on Omarchy the
  agent CLIs are lazy mise launchers that install or self-update (a run took 8 minutes).
  Status probes in jev-code run with `MISE_OFFLINE=1`. Real binaries:
  `~/.local/share/mise/installs/<tool>/<ver>/...`, `~/.local/bin/agy`.
- Don't spend LLM quota in reviews or tests; use the fakes (fake claude, fake Jev via
  `JEV_TYPESAFE_URL`).
- Commit/merge/install only when the user asks (they asked for each merge so far).
- Default-on with a toggle beats opt-in commands.

## 5. How to verify

```sh
cargo build --all-targets            # must be warning-free
cargo test                           # 798 tests at master 98df79a
cargo build --bin code && python3 -I scripts/test_pty_ux.py   # 38/38
python3 -I scripts/test_orch_report.py
bash scripts/install.sh              # installs jev-* into ~/.local/bin, restarts a running gateway
jev-code doctor
```

## 6. Pointers

- Multi-provider design (spec): https://claude.ai/artifact/9LQexcbacp6nbXx9NhYMUH
- Claude Code memory for this project (loaded automatically by Claude sessions here):
  `~/.claude/projects/-home-putraridho-Playground-jev-router/memory/`
- Codex app-server fixtures (redacted): `dashboard/fixtures/codex-app-server/` on `dashboard`.
- Orchestration savings report: `python3 -I scripts/orch_report.py`.
