# AGENTS.md

jev-router: a Rust crate (edition 2021, Rust 1.83+) with two products. `jev-code` is a terminal coding agent that routes each turn across provider CLIs (claude, codex, gemini/agy, grok, deepseek) with failover and orchestration. `jev-claude` (`scripts/jev-claude`) is Claude Code behind a local gateway that switches Haiku/Sonnet/Opus per turn.

## Layout

- `src/lib.rs`: one library crate; every module is declared `pub mod` there (~90 modules, flat in `src/`, with a few dirs: `backend/` provider adapters, `agentview/`, `ext/`, `gimmicks/`).
- `src/bin/`: thin entry points. `code` = jev-code, `gateway` = jev-claude's gateway, `ask`, `chat`, `route`, `tune`, `orch_tune`, `delegate_mcp`.
- `tests/`: integration tests (`perf_budget.rs`, `fixtures/`). Most tests are unit tests inside `src/` modules.
- `scripts/`: `install.sh`, `cached-test.sh`, `test_pty_ux.py` (terminal UX suite), `provider_eval.py` and other Python tools with their own `test_*.py`.
- `mods/jev-orchestrate`: mod loaded by jev-claude. `eval/`: routing eval cases. `docs/`: guide, shortcuts, `architecture/DDD.md` (a plan only, no refactor started).

## Commands

```bash
cargo build --all-targets
scripts/cached-test.sh changed     # quick check: lib tests for files changed since HEAD
scripts/cached-test.sh             # full cargo test (skipped if inputs unchanged since last pass)
scripts/cached-test.sh pty [name]  # PTY UX scenarios against the real binary, fake model CLI (4 in parallel, ~1 min;
                                   # `JEV_PTY_JOBS=n` sets the width; scenarios that passed on this binary are skipped, `JEV_PTY_FRESH=1` reruns)
cargo test <module>::              # one module
./scripts/install.sh               # builds the `local` profile and installs to ~/.local/bin
cargo run --bin code               # run jev-code from source
```

- Use `changed` while iterating; run the full suite and `pty` only at the end.
- CI (`.github/workflows/ci.yml`) builds and tests with `RUSTFLAGS=-D warnings` on Linux and macOS: fix every warning. There is no rustfmt/clippy config.
- Python scripts are run with `python3 -I`. Tests default to 8 threads (`.cargo/config.toml`); many tests create temp git repos, so keep concurrency bounded.
- `JEV_API_KEY` is needed for real runs (copy `.jev-router.env.example` to `~/.jev-router.env`); tests use fakes and need no accounts.

## Release

```bash
# 1. Cargo.toml version (Cargo.lock updates on build); 2. move CHANGELOG `## Unreleased` under `## X.Y.Z`
cargo build --all-targets && scripts/cached-test.sh
git commit -am "Release X.Y.Z"      # tags are optional; the last one is v1.4.0
```

## Code design: SOLID and DRY

All code must follow SOLID and DRY.

- **S, Single Responsibility:** a module, type or function has one reason to change. Example: `workercheck` runs tests; `delegate` decides what to do with the result.
- **O, Open/Closed:** add behaviour by adding code (a new trait impl, a new enum variant handled in one `match`) rather than by editing working code in many places.
- **L, Liskov Substitution:** every implementation of a trait keeps that trait's contract, including what the doc comment promises (for example, "on failure nothing changes"). A test fake must behave like the real implementation.
- **I, Interface Segregation:** keep traits small and focused. A caller should not depend on methods it does not use. Split a growing trait rather than adding unrelated methods to it.
- **D, Dependency Inversion:** core logic depends on traits, not on concrete IO. Git, processes, clocks, the network and disk come in through injected traits (`Worktrees`, `Checks`, `Records`, `Liveness`, `BackendFactory`), so tests can use fakes.
- **DRY, Don't Repeat Yourself:** each piece of knowledge lives in one place. Before writing a helper, look for an existing one and reuse or extend it. When the same logic, constant or string appears twice, extract it. Two things that only look alike but would change for different reasons may stay separate.

## Conventions

- Add a bullet under `## Unreleased` in `CHANGELOG.md` for every user-visible change, written for humans (no review-round labels). Format and release steps: `docs/changelog-guide.md`.
- Bins in `Cargo.toml` with no tests set `test = false`; keep it that way to avoid empty harnesses. No doc-tests.
- README is newcomer-only; put details in `docs/guide.md`.
- Terminal output must stay clean under pipes, `-p`, `--no-color` and `NO_COLOR` (no escape bytes), and respect `JEV_THEME`.
- Treat the transcript as the single canonical history sent to whichever provider a turn routes to.

## Gotchas

- `scripts/install.sh` kills a running `jev-gateway` so it restarts on the new binary.
- Git-backed tests need a git identity (`git config user.email/name`).
- Turn records (`TurnRecord`, `TurnReport`, `audit::Event::Turn`) are built as struct literals in many tests; adding a field means fixing each. New audit fields need `#[serde(default)]` so old `audit.jsonl` lines still read.
- Runtime state, including the audit log, is in `~/.local/state/jev-router/` (`audit.jsonl`: per-turn model, time and tokens; `/bench` reads it).
- `NEXT.md` is a stale handoff note, not a spec; `.claude/` is gitignored.
