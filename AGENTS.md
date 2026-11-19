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
scripts/cached-test.sh pty [name]  # PTY UX scenarios against the real binary, fake model CLI
cargo test <module>::              # one module
./scripts/install.sh               # builds the `local` profile and installs to ~/.local/bin
cargo run --bin code               # run jev-code from source
```

- Use `changed` while iterating; run the full suite and `pty` only at the end.
- CI (`.github/workflows/ci.yml`) builds and tests with `RUSTFLAGS=-D warnings` on Linux and macOS: fix every warning. There is no rustfmt/clippy config.
- Python scripts are run with `python3 -I`. Tests default to 8 threads (`.cargo/config.toml`); many tests create temp git repos, so keep concurrency bounded.
- `JEV_API_KEY` is needed for real runs (copy `.jev-router.env.example` to `~/.jev-router.env`); tests use fakes and need no accounts.

## Conventions

- Add a bullet under `## Unreleased` in `CHANGELOG.md` for every user-visible change, written for humans (no review-round labels). Format and release steps: `docs/changelog-guide.md`.
- Bins in `Cargo.toml` with no tests set `test = false`; keep it that way to avoid empty harnesses. No doc-tests.
- README is newcomer-only; put details in `docs/guide.md`.
- Terminal output must stay clean under pipes, `-p`, `--no-color` and `NO_COLOR` (no escape bytes), and respect `JEV_THEME`.
- Treat the transcript as the single canonical history sent to whichever provider a turn routes to.

## Gotchas

- `scripts/install.sh` kills a running `jev-gateway` so it restarts on the new binary.
- Git-backed tests need a git identity (`git config user.email/name`).
- `NEXT.md` is a stale handoff note, not a spec; `.claude/` is gitignored.
