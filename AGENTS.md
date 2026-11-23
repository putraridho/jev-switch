# AGENTS.md

jev-router: a Rust workspace (edition 2021, Rust 1.83+) with two products. `jev-code` is a terminal coding agent that routes each turn across provider CLIs (claude, codex, gemini/agy, grok, deepseek) with failover and orchestration. `jev-claude` (`scripts/jev-claude`) is Claude Code behind a local gateway that switches Haiku/Sonnet/Opus per turn.

## Layout

- `crates/<context>/`: one library crate per bounded context (`jev-shared`, `jev-catalog`, `jev-approval`, `jev-changes`, `jev-judgment`, `jev-backend`, `jev-routing`, `jev-providers`, `jev-session`, `jev-orchestration`, `jev-doctor`, `jev-chat`, `jev-turn`, `jev-gateway`, `jev-tui`, `jev-calibration`), each with `src/{domain,application,infrastructure,interface}/` and its `lib.rs` as the front door. A crate depends only on the contexts upstream of it in the context map. `crates/testkit` (`jev-testkit`) holds shared test helpers (`fixture!`, `golden!`, scratch dirs, git repos) as a dev-dependency.
- Test helpers another crate's tests need are compiled under `#[cfg(any(test, feature = "testing"))]`; a crate's dev-dependencies turn on its upstream crates' `testing` feature.
- The root package `jev-router` (`src/lib.rs`) is a facade (`pub use jev_<context> as <context>;`) plus the binaries in `src/bin/`: thin entry points. `code` = jev-code, `gateway` = jev-claude's gateway, `ask`, `chat`, `route`, `tune`, `orch_tune`, `delegate_mcp`.
- `tests/`: integration tests (`architecture.rs`: the context map, the layer rule and domain purity; `perf_budget.rs`; `fixtures/`, `golden/`). Most tests are unit tests inside the crates.
- `scripts/`: `install.sh`, `cached-test.sh`, `test_pty_ux.py` (terminal UX suite), `provider_eval.py` and other Python tools with their own `test_*.py`.
- `mods/jev-orchestrate`: mod loaded by jev-claude. `eval/`: routing eval cases. `docs/`: guide, shortcuts, `architecture/DDD.md` (the architecture plan; the layout above follows it).

## Commands

```bash
scripts/check.sh                   # full gate
python3 -I scripts/move_check.py [rev]  # verify a pure move commit
python3 -I scripts/parity.py [rev]       # compare binaries against a revision
cargo build --all-targets
scripts/cached-test.sh changed     # quick check: lib tests for files changed since HEAD
scripts/cached-test.sh             # full cargo test (skipped if inputs unchanged since last pass)
scripts/cached-test.sh pty [name]  # PTY UX scenarios against the real binary, fake model CLI (4 in parallel, ~1 min;
                                   # `JEV_PTY_JOBS=n` sets the width; scenarios that passed on this binary are skipped, `JEV_PTY_FRESH=1` reruns)
cargo test -p jev-<context> <module>::  # one module of one context crate
./scripts/install.sh               # builds the `local` profile and installs to ~/.local/bin
cargo run --bin code               # run jev-code from source
```

- Use `changed` while iterating; run the full suite and `pty` only at the end.
- CI (`.github/workflows/ci.yml`) builds and tests with `RUSTFLAGS=-D warnings` on Linux and macOS: fix every warning. There is no rustfmt/clippy config.
- Python scripts are run with `python3 -I`. Tests default to 8 threads (`.cargo/config.toml`); many tests create temp git repos, so keep concurrency bounded.
- `JEV_API_KEY` is needed for real runs (copy `.jev-router.env.example` to `~/.jev-router.env`); tests use fakes and need no accounts.

## Release

```bash
# 1. Cargo.toml version in [package] and [workspace.package] (Cargo.lock updates on build); 2. move CHANGELOG `## Unreleased` under `## X.Y.Z`
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

## Code design: DDD

All code must follow Domain-Driven Design. The architecture is in `docs/architecture/DDD.md`: a workspace with one crate per context. `tests/architecture.rs` enforces it with an empty baseline (`tests/architecture_baseline/`): T1 the context map, T2 the layer rule (domain and application never import infrastructure), T3 a pure domain (no `serde`, files, environment, processes, network or `Path` methods that ask the disk), and each crate's `Cargo.toml` dependencies follow the context map. Keep the baseline empty.

- **Ubiquitous language:** name types, functions and messages with the domain's words (worker, planner, brief, landing, cooldown, plan usage, turn), the same words the docs and the user use. Do not invent synonyms. Section 1.1 of the plan lists the terms.
- **Bounded contexts:** each area of the domain owns its model, for example catalog, judgment, routing, backend, providers, session, changes, approval, orchestration, turn, doctor and gateway (section 1.2). Code belongs to exactly one context, and a context imports only the contexts upstream of it in the context map (section 1.3), never downstream. No cycles.
- **Layers inside a context:**
  - **Domain:** entities, value objects and rules; pure computation, with time passed in as an argument.
  - **Application:** use cases, plus the ports (traits) they need from outside.
  - **Infrastructure:** adapters that implement the ports, such as CLIs, HTTP, git, files and the clock.
  - **Interface:** delivery, such as the TUI, the MCP server and command output.
  - Dependencies point inward only: domain imports none of the others.
- **Entities and value objects:** an entity has an identity that lasts (a `Worker` with its id). A value object is defined only by its value and is immutable (`ProviderId`, `Tier`, `Landing`). Model states and choices as enums and newtypes, not strings or loose `Value`s.
- **Aggregates:** one root guards its invariants, and others change the aggregate only through the root (for example, the worker registry decides what may be merged).
- **Anti-corruption layers:** external systems (each provider CLI, Jev's API, git, Claude Code's settings) are wrapped by an adapter that turns their formats into domain types. Their wire formats never leak into the domain.
- **Domain services:** a rule that belongs to no single entity is a plain function or service in its context (`pick_worker`, `plan_mode_enforced`), not a method forced onto a type.

## Conventions

- Add a bullet under `## Unreleased` in `CHANGELOG.md` for every user-visible change, written for humans (no review-round labels). Format and release steps: `docs/changelog-guide.md`.
- Bins in `Cargo.toml` with no tests set `test = false`; keep it that way to avoid empty harnesses. No doc-tests (every crate sets `doctest = false`).
- The version lives in two places in the root `Cargo.toml`, `[package]` and `[workspace.package]`; a release bumps both (a test checks they agree).
- README is newcomer-only; put details in `docs/guide.md`.
- Terminal output must stay clean under pipes, `-p`, `--no-color` and `NO_COLOR` (no escape bytes), and respect `JEV_THEME`.
- Treat the transcript as the single canonical history sent to whichever provider a turn routes to.

## Gotchas

- `scripts/install.sh` kills a running `jev-gateway` so it restarts on the new binary.
- Git-backed tests need a git identity (`git config user.email/name`).
- Turn records (`TurnRecord`, `TurnReport`, `audit::Event::Turn`) are built as struct literals in many tests; adding a field means fixing each. New audit fields need `#[serde(default)]` so old `audit.jsonl` lines still read.
- Runtime state, including the audit log, is in `~/.local/state/jev-router/` (`audit.jsonl`: per-turn model, time and tokens; `/bench` reads it).
- The Grok CLI prefers a saved `grok login` (`~/.grok/auth.json`) over `XAI_API_KEY`, and a harness cannot change that (`GROK_CONFIG` refuses auth settings). With a key set and a login present, jev-code runs Grok with `GROK_HOME=~/.local/state/jev-router/grok-key-home`: a folder of symlinks to every `~/.grok` entry except `auth.json*`, re-synced before each turn (`crates/backend/src/infrastructure/grok/grok_cli/key_home.rs`). Never write a login or real files into that folder, and never edit the user's `~/.grok` to force the key.
- `NEXT.md` is a stale handoff note, not a spec; `.claude/` is gitignored.
