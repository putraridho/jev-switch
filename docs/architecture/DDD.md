# jev-router: target architecture (DDD) and migration plan

Status: proposal. Scope: the Rust crate `jev-router` as of commit `6240d92` on `fix-orch-trigger`
(63 `.rs` files, 32,359 lines: about 19,200 production lines and 13,100 test lines, 764 tests).
This document moves no code. Every `file:line` below refers to that commit.

The refactor is **pure**: no behaviour change, no change to binary names, flags, environment
variables, on-disk state, log rows or what any CLI receives on its command line or stdin.

## 0. Goals, in priority order

| # | Goal | What it means here | How it is checked |
|---|------|--------------------|-------------------|
| 1 | **Readability** | From the folder tree and one README, a newcomer finds where any behaviour lives in under a minute. One concept per file, small files, the same file names for the same roles in every context. | Section 9 (budgets, naming, README test), enforced by `scripts/check.sh` |
| 2 | **Strict typing** | No untyped data inside the program. Every external payload becomes a defined struct or enum at the adapter that reads it. No `unwrap`, no stringly-typed states, no `unsafe` outside one audited file. | Section 5, enforced by lints and the architecture test |
| 3 | **DDD, SOLID, DRY** | Bounded contexts with a domain that does no IO, use cases that depend on ports, adapters behind those ports. Each duplication found in section 7 has exactly one home. | Section 3 dependency rule, enforced by the architecture test |
| 4 | **Safety of the change** | Every commit builds without warnings and passes every test. History survives (`git mv`). Any step can be reverted alone. | Section 8 |

Decisions the user must make are collected in section 11. Everything else is decided here.

---

## 1. Domain analysis

### 1.1 Ubiquitous language

One word, one meaning, everywhere: in type names, file names, log fields and screens. The
"today" column shows where the current code uses a different or overloaded name.

| Term | Meaning | Today |
|------|---------|-------|
| **Turn** | One user prompt handled end to end: judge, route, catch up, run, fail over, record. | `turn::Engine::run*` |
| **Attempt** | One try of a turn on one model. A turn has up to `MAX_ATTEMPTS` (6). | `turn::Attempt` |
| **Provider** | A vendor account the user can use: Claude, DeepSeek, Codex, Gemini, Grok. Closed set. | `backend::ProviderId` |
| **Backend** | The adapter that runs one attempt on one provider's CLI and turns its output into Events. | `backend::Backend` |
| **Model** | One model of one provider, identified by a `ModelId`. | bare `String` / `&'static str` |
| **ModelSpec** | Catalog entry for a model: provider, id, tier, billing, context size, task limits. | `catalog::ModelSpec` |
| **Tier** | Capability class of a model: Light, Standard, Strong, Top. | `catalog::Tier`; also a bare `usize` index in `router.rs:23`, `orchestrate.rs:45` |
| **LegacyTier** | The older three-step ladder Haiku/Sonnet/Opus (plus Fable) used by the gateway and by routing-off mode. | `usize` + three copies of a `TIERS` array |
| **TaskType** | What kind of work a turn is (security, hard_debug, review, feature, simple_edit, explain, ...). Jev decides it. | `catalog::TaskType` |
| **Judgment** | Jev's typed answer about one turn: task type, its confidence, and probabilities (edit, security, very hard, continues, correction, multi-part, sequential). | `router::ProviderWant`, `router::CodeWant`, and `signals: serde_json::Value` |
| **Signals** | The numbers of a Judgment, as routing reads them and as the log records them. | `routing::Signals` plus an untyped `Value` copy |
| **Question** | One typed thing asked of Jev: a Choice (pick one of named options) or a Noul (a probability). | `serde_json::Value` built with `json!` |
| **Chain** | The ordered list of models that may answer a TaskType, before availability is applied. | `catalog::default_chain`, `chain_with` |
| **Overrides** | The user's replacement chains (`JEV_CHAINS`). | `catalog::Overrides` |
| **Availability** | What a startup probe learned about a provider: Ready, NotInstalled, NotLoggedIn, Unknown, with detail. | `availability::Availability`, `State` |
| **Cooldown** | A provider is skipped until its usage limit resets. Persisted, shared between processes. | `cooldown::Cooldowns` |
| **Block** | A provider is skipped for this session for a reason that is not a limit (login, unusable directory). | `providers::BlockReason` |
| **PlanUsage** | How full a provider's plan window is, as the CLI last reported it. | `backend::PlanUsage` |
| **Allowances** | What the user permits beyond subscriptions (paid credits, Fable). | `catalog::Allowances` |
| **Usable** | The set of models that can be used right now: available, not cooling, not blocked, not refused. | `catalog::Usable` |
| **ProviderRegistry** | Everything known about providers while the program runs. Answers "who is usable now". | `providers::Runtime` (vague name) |
| **Pin** | The user's override of routing: auto, a provider, or one model. | `routing::Pin` |
| **RouteDecision** | The one model chosen for an attempt, with the reason in plain words. | `routing::RouteDecision` |
| **NoRoute** | No usable model may answer this turn, and why. | `routing::NoRoute` |
| **Hold / Switch gate** | The rule that keeps the current model unless Jev is confident a switch is needed. | `routing::switch_gate`, `hold_within` |
| **Failover** | An attempt failed in a way another model can cover, so the turn moves on. | `turn::Failover` |
| **Event** | One thing a Backend saw in the CLI's stream: tool call, text, finished, failed, plan usage, permission request. | `backend::Event` |
| **ToolCall** | A tool the model ran: name and argument. | a formatted `String` (`"Bash cargo test"`) parsed back in `handoff.rs:225` |
| **Session** | The dialogue of one `jev-code` run: its turn records and each provider CLI's own session id. | `handoff::Sessions` |
| **TurnRecord** | What one finished turn left behind: prompt, reply, who answered, tool calls. | `handoff::TurnRecord` |
| **Handoff** | Sending a turn to a provider that missed earlier turns. | `handoff` |
| **CatchUp** | The plain-text block that brings that provider up to date. Built without any model call. | `handoff::Sessions::catch_up`, `render` |
| **SavedSession** | A session written to disk so `/resume` can continue it. | `handoff::SavedSession` |
| **Approval** | Asking the person whether a tool call may run. | `backend::PermissionRequest`, `Decision`, `Approver` |
| **PermissionRule** | An allow rule in Claude Code's settings file (`Bash(cargo test:*)`). | `String` in `permissions.rs` |
| **Review** | What the person is shown about a tool call before deciding. | `permissions::Review` |
| **Snapshot** | The working tree at one instant (a git tree object plus an untracked-file manifest). | `changes::Snapshot`; `delegate.rs:417` reuses the name for something else |
| **ChangeSet** | What one turn changed: files, counts, and the two Snapshots. | `changes::ChangeSummary` |
| **Undo** | Putting the working tree back to the Snapshot before the turn. | `changes::undo` |
| **Orchestration** | A Planner model splits a task and hands pieces to cheaper Workers. | `orchestrate` + `delegate` |
| **Planner** | The model that plans and reviews. | `orchestrate::choose_planner` |
| **Delegation** | One piece of work handed to a Worker: brief, files, read-only flag. | `delegate::DelegateRequest` |
| **Worker** | A model doing one Delegation in its own Worktree. | `delegate::Worker` |
| **Worktree** | A git worktree created for one Worker. | `delegate::Worktrees` |
| **OrchestrationGate** | Whether a turn gets a Planner (multi-part score, sequential veto, tier). | `orchestrate::Gate`; `routing.rs:208` has another `Gate` |
| **UsageRecord** | One gateway request: tokens, latency, status, tools. | `usage::UsageTap` builds a `Value` |
| **PrefixFingerprint** | Hashes of a request's prefix, used to say where a prompt cache broke. | `prefix::Fp` |
| **Check** | One line of the `doctor` report: severity, detail, fix steps. | `doctor::Check` |

Names that are overloaded today and get exactly one owner: `Decision` (`router.rs:14` and
`backend/mod.rs:164`), `Gate` (`orchestrate.rs:34`, `routing.rs:208`), `Snapshot` (`changes.rs:70`,
`delegate.rs:417`), `SessionInfo` (`handoff.rs:436`, `usage.rs:318`), `Usage` (`anthropic.rs:47`,
`backend/mod.rs:77`), `Interrupt` (`repl.rs:30` trait, `lineedit.rs:35` enum), `run_turn`
(`backend/mod.rs:818`, `repl.rs:83`).

### 1.2 Bounded contexts

Sixteen top-level modules: thirteen domain contexts, the shared kernel, the calibration
tools and the terminal front end. The purpose sentence is the first line of that
context's `README.md` and contains no "and" joining two responsibilities.

| Context | Purpose (README first line) | Aggregates and entities | Value objects | Domain services |
|---------|-----------------------------|-------------------------|---------------|-----------------|
| `shared` | The few types and adapters every context needs. | none | `ProviderId`, `ModelId`, `Probability`, `UnixMillis`, `TokenUsage`, `Secret`, `OpaqueJson` | text truncation, duration wording |
| `catalog` | Which models exist and which may answer a kind of task. | none (static table) | `ModelSpec`, `Tier`, `Billing`, `TaskType`, `LegacyTier`, `Chain`, `Overrides`, `Usable`, `Allowances`, `Pick` | `chain_for`, `pick` |
| `judgment` | Asking Jev typed questions about a turn. | none | `Question`, `Answer`, `Judgment`, `Signals`, `OrchestrationJudgment`, `SecurityLevel` | `Judge` (use case), keyword security level, short follow-up rule |
| `routing` | Choosing the one model for an attempt. | `RoutingState` | `Pin`, `RouteInput`, `RouteDecision`, `NoRoute` | `decide`, switch gate, plan steering, legacy tier route |
| `backend` | Running one attempt on one provider's CLI. | none | `TurnRequest`, `TurnOutcome`, `TurnError`, `ErrorKind`, `Event`, `ToolCall`, `PlanUsage`, `McpAttachment` | error text classification |
| `providers` | Knowing which providers are usable right now. | `ProviderRegistry` (root) with `Cooldowns`, blocks, plan readings | `Availability`, `AvailabilityState`, `BlockReason`, `ResetTime` | per-provider `decide_*` from probe facts, cooldown length, setup guide |
| `session` | Remembering the dialogue across providers. | `Session` (root) with `TurnRecord`s | `CatchUp`, `SavedSession`, `SessionInfo`, `ProjectKey` | catch-up rendering within a token budget |
| `changes` | Knowing what a turn changed in the working tree. | `ChangeSet` | `Snapshot`, `Stamp`, `FileChange`, `ChangeStatus`, `UndoPlan`, `UndoReport` | undo planning, new-file diff |
| `approval` | Deciding whether a tool call may run. | `PermissionSettings` | `PermissionRequest`, `ToolInput`, `Decision`, `Review`, `PermissionRule`, `Refusal`, `AlwaysPlan` | rule policy, visible-text rendering |
| `orchestration` | Splitting a turn between a planner and workers. | `Worker` (root, persisted) | `Delegation`, `DelegationResult`, `BriefSignals`, `FilePatch`, `OrchestrationGate`, `PlannerChoice` | worker pick, patch paging, recovery fate |
| `turn` | Carrying one turn from prompt to report. | `Engine` (application service holding the session's state) | `TurnReport`, `TurnStatus`, `Judged`, `Attempt`, `Failover`, `Cancel` | none (it coordinates the other contexts) |
| `doctor` | Telling the user what is not set up. | `Report` | `Check`, `Severity`, `Inputs` | `diagnose`, first-run hint |
| `gateway` | Serving Claude Code through an Anthropic-format proxy. | `GatewayTurn` (usage roll-up per user turn), `SessionState` | `UsageRecord`, `ModelChoice`, `PrefixFingerprint` | prompt text rules, Haiku adaptation, similarity |
| `chat` | The plain question-and-answer router (`jev-chat`). | `Transcript` | `Message`, `Role` | none |
| `calibration` | Reading thresholds off labeled or logged prompts. | none | `Scored`, `Cutoff`, `PromptItem` | threshold sweep |
| `tui` | The terminal front end of `jev-code`. | none (interface only) | screen facts (`RouteFacts`, `StatusFacts`, ...) | formatters |

`tui` is not a domain context: it is the delivery layer for `turn`, and it renders facts from
several contexts. It has no `domain/`.

### 1.3 Context map

Arrows point from the dependent (downstream) context to the one it depends on (upstream).
A context may import only the contexts listed in its row; the architecture test holds this
table as data (section 3.4).

```
                                   shared  (kernel: everyone depends on it)
                                      ^
        catalog <------ judgment      |            approval
           ^  ^            ^          |               ^
           |  +--------+   |          |               |
           |           routing     providers <---- backend
           |              ^           ^   ^           ^
           |              |           |   |           |
   session |   changes    |           |   |           |
      ^    |      ^       |           |   |           |
      +----+------+-------+-----------+---+-----------+
                          |
                        turn  <----------- orchestration (planner half is called by turn;
                          ^                 worker half reuses backend, providers, catalog, judgment)
                          |
              +-----------+-----------+
             tui        doctor      (embed facade for Flashboard)

   gateway  --> judgment, catalog, shared          chat --> judgment, catalog, shared
   calibration --> judgment, routing, catalog, providers, backend, gateway (log reader), shared
```

| Downstream | Upstream (allowed imports) | Relationship |
|------------|----------------------------|--------------|
| `catalog` | `shared` | Shared kernel |
| `judgment` | `catalog`, `shared` | Conformist to `TaskType`; **Jev is an external service behind the `JudgmentService` port**, and `infrastructure/typesafe_http.rs` is its anti-corruption layer |
| `routing` | `judgment`, `catalog`, `shared` | Customer of `judgment` (reads `Signals`) |
| `approval` | `shared` | **ACL to Claude Code's settings file** (`infrastructure/claude_settings_file.rs`) |
| `backend` | `approval`, `shared` | **One ACL per provider CLI** (`claude`, `codex`, `gemini`/`agy`, `grok`); DeepSeek reuses the Claude ACL with different environment |
| `providers` | `backend`, `catalog`, `shared` | Customer of `backend` (`ErrorKind`, `PlanUsage`); ACLs to each CLI's login probe |
| `session` | `shared` | Published language: `TurnRecord` |
| `changes` | `shared` | **ACL to git** |
| `orchestration` | `backend`, `providers`, `catalog`, `judgment`, `shared` | Customer; ACL to git worktrees; open-host service to planners through the MCP server |
| `turn` | all of the above | The core use case; nothing below depends on it |
| `doctor` | `providers`, `judgment`, `shared` | Customer |
| `gateway` | `judgment`, `catalog`, `shared` | ACL to the Anthropic HTTP API |
| `chat` | `judgment`, `catalog`, `shared` | Customer |
| `calibration` | `judgment`, `routing`, `catalog`, `providers`, `backend`, `gateway`, `shared` | Customer (read-only tools) |
| `tui` | `turn`, `doctor`, and the root API of every context whose facts it shows | Interface |

Cycles that exist today and disappear: `turn -> orchestrate -> delegate -> turn`
(`delegate.rs:61` imports `turn::{idle_timeout_from_env, BackendFactory}`), `doctor <-> uifmt`
(`uifmt/failure.rs:33` calls `doctor::login_instruction`, `doctor.rs:943` calls
`uifmt::next_steps_for`), `commands -> uifmt -> backend` for a module documented as pure.

---

## 2. Layers inside a context

Every context has the same four folders. A folder is absent when it would be empty (`routing`
and `catalog` are almost all domain).

| Layer | Folder | Holds | May do |
|-------|--------|-------|--------|
| Domain | `domain/` | Entities, value objects, rules, domain services. | Pure computation. Time and randomness arrive as arguments. |
| Application | `application/` | Use cases, and `ports.rs` with the traits those use cases need from the outside. | Call the domain, call ports, be `async`. |
| Infrastructure | `infrastructure/` | Adapters that implement ports: CLIs, HTTP, files, git, the clock. Wire and file formats (DTOs). | Any IO. |
| Interface | `interface/` | Delivery: HTTP handlers, the MCP server, one-shot command output. | Parse input, call a use case, format output. |

Binaries in `src/bin/` are the composition roots: they read configuration, build adapters, hand
them to use cases, and contain nothing else (30 lines or fewer each).

---

## 3. The dependency rule and its enforcement

### 3.1 The rule

1. **Inward only.** Inside a context: `interface -> application -> domain`, and
   `infrastructure -> application, domain`. `domain` imports nothing from the other three.
   `application` never imports `infrastructure` or `interface`.
2. **Through the front door.** Another context is imported only as `crate::<context>::Name`.
   A path that goes into another context's layer folder
   (`crate::routing::domain::...`) is forbidden.
3. **Along the map.** Context A may import context B only if section 1.3 lists B for A.
4. **Layer-preserving across contexts.** A `domain` file may import from another context only
   names that originate in that context's `domain`. An `application` file may import its
   `domain` and `application` names. Adapters of another context are reachable only as
   `crate::<context>::adapters::Name`, and only from `src/bin/*` and files named `compose.rs`.
5. **Outside crates by layer:**

| Layer | Allowed | Forbidden |
|-------|---------|-----------|
| `domain` | `std` except the modules on the right | `std::fs`, `std::process`, `std::env`, `std::net`, `std::io` (other than `std::io::Error` as a field type), `std::time::SystemTime::now`, `tokio`, `reqwest`, `axum`, `rustyline`, `libc`, `serde`, `serde_json`, `anyhow`, `futures_util` |
| `application` | `std` (same exclusions), `tokio::sync`, `tokio::time`, `tokio::select!`, `std::future` | `tokio::process`, `tokio::fs`, `tokio::net`, `reqwest`, `axum`, `rustyline`, `libc`, `serde`, `serde_json`, `anyhow` |
| `infrastructure` | everything in `Cargo.toml` except `anyhow`, `axum`, `rustyline` | those three |
| `interface` | `axum` (gateway), `rustyline` (tui), `serde`/`serde_json` for request and response DTOs | `reqwest`, `libc`, `anyhow` |
| `src/bin/*` | `anyhow` allowed here only | logic |

   `serde` is forbidden in `domain` on purpose: a file or log format must not change because a
   domain field was renamed. Formats are DTO structs in `infrastructure` with `From`
   conversions, and golden tests pin their bytes.

### 3.2 Why one crate (recommended) and not a workspace of crates

| Option | For | Against |
|--------|-----|---------|
| **A. One package `jev-router`, contexts as modules, rule enforced by a test (recommended)** | `scripts/install.sh`, `scripts/provider_eval.py` (`target/release/ask`) and `scripts/test_pty_ux.py` (`target/debug/code`) keep working unchanged. The Flashboard design already fixes the layout: a workspace rooted here with `default-members = ["."]` and the engine reused through `jev_router::embed`. One compile unit for eight binaries. `pub(crate)` keeps test seams (`Runtime::for_tests`) private. Each step is a file move, so `git log --follow` works. | The compiler does not enforce the rule; a test does. `#![forbid(unsafe_code)]` cannot be crate-wide because one file needs `unsafe` (section 5.5). |
| B. One crate per context (16 crates) | The compiler enforces direction. `forbid(unsafe_code)` in 15 crates. Parallel compilation. | Every cross-context item becomes `pub` API. Test doubles shared across contexts need a seventeenth crate or feature flags. Eight binaries link sixteen rlibs. The move is no longer mechanical. Flashboard's `embed` plan needs rewriting. Roughly double the migration risk for the same readability. |
| C. Three crates by layer (`domain`, `application`, `infrastructure`) | Compiler-enforced layers. | Splits every context across three crates, which destroys the "one folder per context" readability that is goal 1. |

Recommendation: **A now**. The structure is crate-ready: each context exposes its surface in
one `mod.rs` and is imported only through it, so promoting a context to a crate later is a
directory move plus a `Cargo.toml`. If the user wants compiler enforcement, do it after the
definition of done is met, starting with `shared` (which would also make `forbid(unsafe_code)`
possible everywhere else). This is decision U1.

### 3.3 Visibility

- Layer modules are private to the context: `mod domain; mod application; mod infrastructure;`
  in the context's `mod.rs`. Other contexts cannot name them at all, so rule 2 is also a
  compiler error.
- Items are `pub(crate)` unless the context's `mod.rs` re-exports them. A re-export is an
  explicit list, never a glob.
- The only `pub` items of the crate are those re-exported by `src/embed.rs` (Flashboard) and
  those the binaries use. `unreachable_pub` is denied, so a stray `pub` is a build failure.

### 3.4 The architecture test

`tests/architecture.rs` (an integration test, standard library only, no new dependency). It
reads every file under `src/`, removes comments, string literals and `#[cfg(test)]` modules,
and collects every `crate::a::b::...` path (in `use` items and inline). Three small source
rules make this exact without a Rust parser:

- every cross-module path starts with `crate::` (no `super::super::`; `super::` only inside
  one layer folder),
- one `use` item names one context (`use crate::routing::{Pin, RouteDecision};`),
- no glob imports outside `#[cfg(test)]`.

The test fails, naming file, line and rule, on:

| Check | Rule |
|-------|------|
| A1 | a layer imports a layer it must not (3.1 rule 1) |
| A2 | a path enters another context's layer folder (rule 2) |
| A3 | a context imports a context not in its row of the map (rule 3); the map is a `const` table in the test |
| A4 | a cross-context name comes from the wrong layer (rule 4); the test builds a name-to-layer index from each context's `mod.rs` re-export list |
| A5 | a forbidden outside crate or `std` module for the file's layer (rule 5) |
| A6 | `serde_json::Value`, `serde_json::Map` or `json!` outside the allow-listed wire files (section 5.2) |
| A7 | the token `unsafe` outside `shared/infrastructure/sys_unix.rs` |
| A8 | `std::env::var`, `var_os` or `current_exe` outside `shared/infrastructure/env.rs` |
| A9 | `Secret::expose` outside `infrastructure/` |
| A10 | a `mod.rs` that contains anything but docs, `mod` items and `pub use` lists |
| A11 | a context folder without `README.md`, or a module without a `//!` doc comment |
| A12 | file names from the banned list (`util`, `utils`, `helpers`, `misc`, `common`, `types`, `stuff`) |

**Ratchet.** The test starts with `tests/architecture_baseline.txt`: the list of current
violations, one per line. A violation in the baseline passes; a new one fails; a baseline line
that no longer matches also fails (so the list can only shrink, and is deleted last). This is
what lets the migration run in small green steps.

### 3.5 The one gate

`scripts/check.sh` is the single command every step runs, locally and in CI:

```
cargo build --all-targets            (RUSTFLAGS=-D warnings)
cargo clippy --all-targets -- -D warnings
cargo test                           (includes tests/architecture.rs and tests/embed_api.rs)
python3 -I scripts/budget.py         (file and function size, section 9.2)
cargo build --bin code && python3 -I scripts/test_pty_ux.py
python3 -I scripts/test_orch_report.py
python3 -I scripts/test_task_type_accuracy.py
python3 -I -m unittest scripts/test_provider_eval.py
```

None of these starts a real provider CLI or spends quota: the Rust tests use fake `sh` scripts
and a fake Jev server, the PTY test uses a fake `claude`.

---

## 4. Target folder tree

### 4.1 Conventions

| Topic | Rule |
|-------|------|
| Context folder | `src/<context>/` with `README.md`, `mod.rs`, `error.rs`, then `domain/`, `application/`, `infrastructure/`, `interface/` |
| `mod.rs` vs `name.rs` | A folder module always uses `mod.rs` (the folder is self-contained, and this is what `backend/` and `uifmt/` already do). A `mod.rs` holds only the `//!` doc, `mod` declarations and the explicit `pub use` list. Logic never lives in `mod.rs`. |
| File name | A singular noun from the glossary for a type (`snapshot.rs`, `route_decision.rs`), a verb phrase for a use case (`undo_turn.rs`, `resume_session.rs`). One concept per file. |
| Same role, same name | `ports.rs` (traits a layer needs), `error.rs` (the context's error), `config.rs` (the context's typed settings and how they are read), `<thing>_wire.rs` (serde DTOs for a format), `<thing>_file.rs` (a file adapter), `<thing>_http.rs` (an HTTP adapter), `<thing>_cli.rs` (a CLI adapter), `git_<thing>.rs` (a git adapter), `compose.rs` (wiring) |
| Banned names | `util`, `utils`, `helpers`, `misc`, `common`, `types`, `manager`, `stuff` |
| Module doc | Every file starts with `//!` saying what it **owns** and what it **must not** do (for example: "Owns the cooldown length rules. Must not read the clock or the disk."). |
| Errors | One `Error` enum per context in `<context>/error.rs`, hand-written `Display` and `std::error::Error` (no new dependency). Variants are closed and typed; no `String`-only "other" variant except where the text is a provider's own message. `anyhow` only in `src/bin/*`. |
| Unit tests | In the file, `#[cfg(test)] mod tests`, when the file stays within the size budget. Otherwise in `<name>_tests.rs` beside it (`#[cfg(test)] #[path = "<name>_tests.rs"] mod tests;`), which keeps access to private items. |
| Use-case tests | `application/tests/` in the context (`mod.rs` holds the fakes, one file per behaviour). |
| Test doubles shared by contexts | `src/testkit/` (`#[cfg(test)]`): scratch directory, temporary git repository, scripted fake CLI, fake Jev server, capture writer. Replaces the seven private copies listed in 7.2. |
| Fixtures | `tests/fixtures/<cli>/` stays where it is. Tests load them with `fixture!("claude/turn1.jsonl")`, which expands to `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", ...))`, so moving a source file never breaks a path. |
| Cross-context tests | `tests/` (integration): `architecture.rs`, `embed_api.rs`, `formats.rs` (golden bytes of every file and log format). |

### 4.2 Cross-cutting concerns

| Concern | One home | Replaces |
|---------|----------|----------|
| Environment variables and the env file | `shared/infrastructure/env.rs`: the only caller of `std::env`. A closed `EnvVar` enum lists every variable the program reads (about 57 names today). Each context has a `config.rs` that turns `&dyn Env` into its typed `Config` once, at startup. | 44 direct `std::env` reads in 16 files and 31 `envfile::lookup` calls in 16 files |
| Secrets | `shared/domain/secret.rs`: `Secret` wraps a key; `Debug` and `Display` print `[redacted]`; it has no `Serialize`; `expose()` is callable only from `infrastructure/` (check A9). Every API key travels as `Secret`. | bare `String` keys (`providers.rs:43`, `typesafe.rs:151`) |
| State directory | `shared/infrastructure/state_dir.rs` behind the `StateDir` port. | `log::state_dir()` called from 8 modules |
| Clock | `shared/application/ports.rs`: `trait Clock { fn now(&self) -> UnixMillis; }`, `shared/infrastructure/system_clock.rs`. | `log::now_ms` (`log.rs:28`), `doctor::now_ms` (`doctor.rs:543`), two `Clock` closure aliases (`turn.rs:281`, `delegate.rs:979`) |
| JSON-lines logs | `shared/infrastructure/jsonl_log.rs`: append with rotation, generic over `T: Serialize`. | `log::append(&Value)` |
| Private atomic file writes | `shared/infrastructure/private_file.rs`: write to a temporary file with mode 0600, then rename. | five copies (7.2, row D3) |
| git | `shared/infrastructure/git_command.rs`: one runner (directory, optional index, optional stdin, typed failure). | four runners (7.2, row D1) |
| Unsafe system calls | `shared/infrastructure/sys_unix.rs` (section 5.5). | `unsafe` in three files |
| Poison-tolerant locking | `shared/domain/lock.rs`: `lock(&Mutex<T>)`. | nine hand-written copies (7.2, row D7) |

### 4.3 The tree

```
jev-router/
├── Cargo.toml                    [workspace] root, default-members = ["."]; [lints] table (section 5.6)
├── clippy.toml                   disallowed types and macros, test allowances
├── README.md
├── docs/architecture/DDD.md      this document
├── eval/  mods/                  unchanged
├── scripts/
│   ├── install.sh                unchanged (same --bin names, same installed names)
│   ├── check.sh                  the one gate (3.5)                               NEW
│   ├── budget.py                 size budgets (9.2)                               NEW
│   ├── parity.sh                 old-binary vs new-binary output comparison (8.4) NEW
│   └── ...                       python evals and tests, unchanged
├── tests/
│   ├── architecture.rs           the dependency rule (3.4)                        NEW
│   ├── architecture_baseline.txt the ratchet, deleted at the end                  NEW
│   ├── embed_api.rs              compiles one use of every name in embed.rs       NEW
│   ├── formats.rs                golden bytes of every file, log and wire format  NEW
│   └── fixtures/                 unchanged
└── src/
    ├── lib.rs                    crate doc, lint attributes, the 16 contexts in dependency order
    ├── embed.rs                  the stable API for Flashboard: re-exports only
    ├── testkit/                  #[cfg(test)] shared test doubles
    │
    ├── shared/
    │   ├── README.md  mod.rs
    │   ├── domain/           provider_id.rs  model_id.rs  probability.rs  unix_millis.rs
    │   │                     token_usage.rs  secret.rs  opaque_json.rs  truncation.rs
    │   │                     duration_text.rs  lock.rs
    │   ├── application/      ports.rs                      (Clock, Env, StateDir)
    │   └── infrastructure/   env.rs  env_file.rs  state_dir.rs  system_clock.rs  jsonl_log.rs
    │                         private_file.rs  git_command.rs  sys_unix.rs
    │
    ├── catalog/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           task_type.rs  tier.rs  legacy_tier.rs  model_spec.rs  model_table.rs
    │   │                     chain.rs  overrides.rs  usable.rs  allowances.rs  pick.rs
    │   └── infrastructure/   chains_file.rs  chains_wire.rs
    │
    ├── judgment/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           question.rs  answer.rs  judgment.rs  signals.rs  thresholds.rs
    │   │                     question_catalog.rs  orchestration_judgment.rs
    │   │                     security_keywords.rs  short_follow_up.rs  tier_want.rs
    │   ├── application/      ports.rs  judge.rs  judge_worker.rs  verify_answer.rs
    │   └── infrastructure/   typesafe_http.rs  typesafe_wire.rs  config.rs
    │
    ├── routing/
    │   ├── README.md  mod.rs
    │   └── domain/           pin.rs  route_input.rs  routing_state.rs  route_decision.rs
    │                         decide.rs  pinned_choice.rs  auto_choice.rs  switch_gate.rs
    │                         plan_steer.rs  legacy_tier_route.rs
    │
    ├── approval/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           permission_request.rs  tool_input.rs  decision.rs  review.rs
    │   │                     visible_text.rs  permission_rule.rs  bash_rule.rs  rule_policy.rs
    │   │                     always_plan.rs  listing.rs
    │   ├── application/      ports.rs  remember_rule.rs  list_rules.rs
    │   └── infrastructure/   claude_settings_file.rs  claude_settings_wire.rs  directory_path.rs
    │
    ├── backend/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           turn_request.rs  turn_outcome.rs  turn_error.rs  error_kind.rs
    │   │                     error_text.rs  limit_text.rs  event.rs  tool_call.rs  plan_usage.rs
    │   │                     mcp_attachment.rs  prompt_arg.rs
    │   ├── application/      ports.rs  ask_once.rs       (TurnRunner, Approver, BackendFactory)
    │   ├── infrastructure/
    │   │   ├── cli_dialect.rs  cli_turn_runner.rs  backend_factory.rs  config.rs
    │   │   ├── process/      child.rs  output_pump.rs  stdin_feed.rs  stderr_tail.rs
    │   │   │                 process_group.rs  prompt_files.rs  idle_watchdog.rs
    │   │   ├── claude/       mod.rs  claude_cli.rs  claude_wire.rs  permission_wire.rs  mcp_config.rs
    │   │   ├── codex/        mod.rs  codex_cli.rs  codex_wire.rs
    │   │   ├── gemini/       mod.rs  gemini_cli.rs  gemini_wire.rs  agy_wire.rs  flavor.rs
    │   │   ├── grok/         mod.rs  grok_cli.rs  grok_wire.rs
    │   │   └── deepseek/     mod.rs  deepseek_env.rs
    │   └── interface/        ask_output.rs                (the one JSON line `jev-ask` prints)
    │
    ├── providers/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           availability.rs  cooldowns.rs  cooldown_length.rs  reset_time.rs
    │   │                     block_reason.rs  reaction.rs  allowances_rule.rs  setup_guide.rs
    │   │                     late_probe.rs
    │   │                     probe_facts/  claude.rs  codex.rs  gemini.rs  grok.rs  deepseek.rs
    │   ├── application/      ports.rs  provider_registry.rs  probe_providers.rs  retry_slow_probes.rs
    │   └── infrastructure/   cooldown_file.rs  cooldown_wire.rs  system_date.rs  path_lookup.rs
    │                         mise_install.rs  config.rs
    │                         probes/  claude_probe.rs  codex_probe.rs  codex_auth_wire.rs
    │                                  gemini_probe.rs  grok_probe.rs  deepseek_probe.rs
    │
    ├── session/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           session.rs  turn_record.rs  catch_up.rs  catch_up_level.rs
    │   │                     tool_trail.rs  saved_session.rs  session_info.rs  project_key.rs
    │   ├── application/      ports.rs  save_session.rs  resume_session.rs
    │   └── infrastructure/   session_file.rs  session_wire.rs
    │
    ├── changes/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           snapshot.rs  file_change.rs  change_set.rs  untracked_merge.rs
    │   │                     undo_plan.rs  undo_report.rs  new_file_diff.rs  safe_path.rs
    │   ├── application/      ports.rs  track_turn.rs  undo_turn.rs  show_diff.rs
    │   └── infrastructure/   git_working_tree.rs  git_temp_index.rs  git_output.rs  undo_files.rs
    │
    ├── orchestration/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           orchestration_gate.rs  planner_choice.rs  planner_note.rs
    │   │                     brief_signals.rs  worker_pick.rs  worker.rs  worker_prompt.rs
    │   │                     delegation.rs  delegation_result.rs  file_patch.rs  patch_page.rs
    │   │                     recovery.rs
    │   ├── application/      ports.rs  delegator.rs  delegate_work.rs  run_worker.rs
    │   │                     show_worker_diff.rs  merge_worker.rs  discard_worker.rs
    │   │                     recover_workers.rs  reservation.rs  attach_delegate_tool.rs
    │   ├── infrastructure/   git_worktrees.rs  worktree_base.rs  worker_file.rs  worker_wire.rs
    │   │                     process_liveness.rs  mcp_config_file.rs  delegate_log.rs  config.rs
    │   └── interface/mcp/    mod.rs  json_rpc.rs  tool_schema.rs  tool_arguments.rs
    │                         mcp_server.rs  lazy_delegator.rs  compose.rs
    │
    ├── turn/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           turn_report.rs  judged.rs  attempt.rs  failover.rs  no_route_info.rs
    │   ├── application/      ports.rs  engine.rs  engine_builder.rs  cancel.rs  run_turn.rs
    │   │                     run_attempts.rs  choose_model.rs  plan_orchestration.rs
    │   │                     run_attempt.rs  finish_turn.rs  approve_tool.rs
    │   │                     session_commands.rs  change_commands.rs
    │   └── infrastructure/   turn_log.rs  turn_log_wire.rs  git_status.rs  config.rs
    │
    ├── doctor/
    │   ├── README.md  mod.rs
    │   ├── domain/           check.rs  report.rs  inputs.rs  diagnose.rs  provider_checks.rs
    │   │                     machine_checks.rs  first_run_hint.rs
    │   ├── application/      ports.rs  gather_inputs.rs
    │   └── infrastructure/   machine_facts.rs
    │
    ├── gateway/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           model_choice.rs  prompt_text.rs  harness_message.rs  recent_context.rs
    │   │                     haiku_adaptation.rs  session_state.rs  usage_record.rs
    │   │                     gateway_turn.rs  similarity.rs  prefix_fingerprint.rs  cache_break.rs
    │   ├── application/      ports.rs  choose_model.rs  forward_request.rs  check_orchestration.rs
    │   │                     check_worker.rs  check_outcome.rs  usage_tap.rs  turn_rollup.rs
    │   │                     prefix_watch.rs
    │   ├── infrastructure/   anthropic_upstream_http.rs  anthropic_wire.rs  sse_wire.rs
    │   │                     usage_log.rs  usage_log_wire.rs  decision_log_wire.rs  config.rs
    │   └── interface/http/   mod.rs  routes.rs  proxy_handler.rs  check_handlers.rs  compose.rs
    │
    ├── chat/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           message.rs  transcript.rs
    │   ├── application/      ports.rs  chat_turn.rs
    │   ├── infrastructure/   claude_print_cli.rs  claude_print_wire.rs
    │   └── interface/        chat_repl.rs
    │
    ├── calibration/
    │   ├── README.md  mod.rs  error.rs
    │   ├── domain/           threshold_sweep.rs  prompt_item.rs  score_band.rs
    │   ├── application/      sweep_cheap_threshold.rs  dry_run_routes.rs  sweep_orchestration_cutoff.rs
    │   └── infrastructure/   cases_file.rs  prompt_file.rs  gateway_log_reader.rs
    │
    ├── tui/                  interface only: the terminal front end of jev-code
    │   ├── README.md  mod.rs
    │   ├── app/              mod.rs  compose.rs  interactive_session.rs  print_mode.rs
    │   │                     startup.rs  doctor_command.rs
    │   ├── cli_args.rs  command.rs  command_table.rs  permissions_command.rs
    │   ├── repl/             mod.rs  repl.rs  turn_race.rs  command_handlers.rs  pager.rs
    │   ├── console/          mod.rs  console.rs  status_ticker.rs  turn_observer.rs
    │   │                     approval_prompt.rs
    │   ├── input/            mod.rs  line_editor.rs  completion_helper.rs  history_file.rs
    │   │                     interrupt_guard.rs  answer_key.rs
    │   ├── views/            mod.rs  provider_rows.rs  status_facts.rs  change_rows.rs
    │   │                     summary_facts.rs  banner.rs  prompt_label.rs  doctor_items.rs
    │   │                     session_lines.rs  first_run.rs  exit_code.rs
    │   ├── render/           (today's uifmt, file for file) ansi.rs  changes.rs  completion.rs
    │   │                     failure.rs  markdown.rs  permissions.rs  provider_view.rs  route.rs
    │   │                     screens.rs  status.rs  style.rs  summary.rs  text.rs  width.rs
    │   │                     wrap.rs  providers_text.rs
    │   └── tty/              mod.rs  terminal_guard.rs  terminal_size.rs  screen.rs  panic_hook.rs
    │
    └── bin/                  composition roots only, 30 lines or fewer each
        ├── code.rs           -> tui::app::run()                         installed as jev-code
        ├── gateway.rs        -> gateway::interface::http                installed as jev-gateway
        ├── delegate_mcp.rs   -> orchestration::interface::mcp           installed as jev-delegate-mcp
        ├── ask.rs            -> backend ask_once                        installed as jev-ask
        ├── chat.rs           -> chat::interface                         installed as jev-chat
        └── tune.rs  route.rs  orch_tune.rs   -> calibration
```

`[[bin]]` names and paths in `Cargo.toml` do not change, so `scripts/install.sh`,
`orchestration`'s search for `delegate_mcp` beside the executable (`orchestrate.rs:151`) and
both Python scripts that name `target/.../code` and `target/release/ask` keep working.

### 4.4 Public API

- `src/embed.rs` is the stable surface for Flashboard, exactly as the dashboard design names it
  (`jev_router::embed`). It contains re-exports only. The names the dashboard design lists stay:
  `ProviderId`, `ALL_PROVIDERS`, `Runtime` (kept as an alias of `ProviderRegistry`),
  `probe_all`, `Availability`, `AvailabilityState`, `state_dir`, `path_from_env`,
  `env_file_path`, `parse_env`, `install_hint`, `login_command`. For the future chat window it
  adds `Engine`, `EngineBuilder`, `TurnObserver`, `TurnReport`, `TurnStatus`, `Cancel`, `Event`,
  `PermissionRequest`, `Decision`, `Pin`, `ChangeSet`, `SessionInfo`.
- `tests/embed_api.rs` uses every one of those names, so removing or renaming one is a failing
  test, not a surprise in the app.
- Binaries import context roots (`jev_router::turn::Engine`), never layer folders.

### 4.5 Every current file and where it goes

"Split" means the file becomes several files in the listed folder; the first file named keeps
the `git mv` history (it holds the largest part).

| Current path | Lines | New path | Split or merge, and why |
|--------------|------:|----------|-------------------------|
| `src/lib.rs` | 41 | `src/lib.rs` | Lists contexts instead of 30 flat modules |
| `src/envfile.rs` | 121 | `shared/infrastructure/env_file.rs` | Pure parsing stays with it; `lookup` becomes the `Env` port in `env.rs` |
| `src/log.rs` | 59 | `shared/infrastructure/jsonl_log.rs` | Split: `state_dir` to `state_dir.rs`, `now_ms` to `system_clock.rs`. It mixed three concerns |
| `src/typesafe.rs` | 188 | `judgment/infrastructure/typesafe_http.rs` | Split: request and response shapes to `typesafe_wire.rs`; `ChoiceAnswer` to `judgment/domain/answer.rs` |
| `src/router.rs` | 1197 | `judgment/application/judge.rs` | Split, see 7.3 |
| `src/catalog.rs` | 576 | `catalog/domain/model_table.rs` | Split by concept (`task_type`, `tier`, `model_spec`, `chain`, `overrides`, `usable`, `allowances`, `pick`); `Overrides::from_env` (reads a file, `catalog.rs:252`) to `infrastructure/chains_file.rs` |
| `src/routing.rs` | 872 | `routing/domain/decide.rs` | Split: `pin`, `route_input`, `routing_state`, `route_decision`, `pinned_choice`, `auto_choice`, `switch_gate`, `plan_steer` |
| `src/anthropic.rs` | 226 | `chat/infrastructure/claude_print_cli.rs` | Split three ways: model id constants to `catalog/domain/legacy_tier.rs`; `is_limit_phrase`/`is_limit_message` to `backend/domain/limit_text.rs`; `Message` to `chat/domain/message.rs`. A chat client, a model list and an error classifier shared one file |
| `src/transcript.rs` | 33 | `chat/domain/transcript.rs` | Move |
| `src/backend/mod.rs` | 1874 | `backend/infrastructure/process/child.rs` | Split, see 7.3 |
| `src/backend/claude.rs` | 564 | `backend/infrastructure/claude/claude_cli.rs` | Split: typed events to `claude_wire.rs`, approval lines to `permission_wire.rs`, MCP config to `mcp_config.rs` |
| `src/backend/codex.rs` | 482 | `backend/infrastructure/codex/codex_cli.rs` | Split: `codex_wire.rs` |
| `src/backend/gemini.rs` | 718 | `backend/infrastructure/gemini/gemini_cli.rs` | Split: `gemini_wire.rs`, `agy_wire.rs`, `flavor.rs` (two CLIs, one provider) |
| `src/backend/grok.rs` | 408 | `backend/infrastructure/grok/grok_cli.rs` | Split: `grok_wire.rs` |
| `src/providers.rs` | 904 | `providers/application/provider_registry.rs` | Split: backend construction (`backend_for`, `deepseek_env`, gemini flavor, lines 43-116) to `backend/infrastructure/backend_factory.rs`; `react` rules to `domain/reaction.rs`; late probes to `retry_slow_probes.rs`; `render*` and `format_left` to `tui/render/providers_text.rs` and `shared/domain/duration_text.rs` |
| `src/availability.rs` | 882 | `providers/domain/availability.rs` | Split: each provider's pure `decide_*`/`parse_*` to `domain/probe_facts/<provider>.rs`, its IO half to `infrastructure/probes/<provider>_probe.rs`; PATH and mise checks to `path_lookup.rs`, `mise_install.rs`; `render*` to `tui/render` |
| `src/cooldown.rs` | 852 | `providers/domain/reset_time.rs` | Split: `cooldown_length.rs`, `cooldowns.rs` (the set), `infrastructure/cooldown_file.rs` (save, load, `DiskWatch`), `infrastructure/system_date.rs` (the `date` command and zoneinfo) |
| `src/handoff.rs` | 1105 | `session/domain/catch_up.rs` | Split: `session.rs` (`Sessions`), `turn_record.rs`, `tool_trail.rs`, `saved_session.rs`, `session_info.rs`, `project_key.rs`; file reading and writing to `infrastructure/session_file.rs`; `write_atomic` to `shared/infrastructure/private_file.rs` |
| `src/changes.rs` | 1586 | `changes/infrastructure/git_working_tree.rs` | Split, see 7.3 |
| `src/permissions.rs` | 1348 | `approval/domain/rule_policy.rs` | Split, see 7.3 |
| `src/orchestrate.rs` | 303 | `orchestration/domain/orchestration_gate.rs` | Split: `planner_choice.rs`, `planner_note.rs`; `Setup` to `application/attach_delegate_tool.rs`; `write_mcp_config` to `infrastructure/mcp_config_file.rs` |
| `src/delegate.rs` | 2856 | `orchestration/application/delegator.rs` | Split, see 7.3 |
| `src/turn.rs` | 2743 | `turn/application/engine.rs` | Split, see 7.3 and 9.4 |
| `src/turnlog.rs` | 152 | `turn/infrastructure/turn_log.rs` | Split: row shape to `turn_log_wire.rs`; `ask_json` to `backend/interface/ask_output.rs` |
| `src/doctor.rs` | 952 | `doctor/domain/diagnose.rs` | Split: `check.rs`, `report.rs`, `inputs.rs`, `provider_checks.rs`, `machine_checks.rs`, `first_run_hint.rs`; `gather` to `application/gather_inputs.rs`; machine probing to `infrastructure/machine_facts.rs`; `install_hint`, `login_command`, `login_instruction` to `providers/domain/setup_guide.rs` together with `providers::login_target` (they are provider knowledge, and this removes the `doctor <-> uifmt` cycle) |
| `src/usage.rs` | 676 | `gateway/application/usage_tap.rs` | Split: `turn_rollup.rs` (`Turns`), domain `usage_record.rs`, `gateway_turn.rs`, `similarity.rs`; record shapes to `infrastructure/usage_log_wire.rs` |
| `src/prefix.rs` | 243 | `gateway/domain/prefix_fingerprint.rs` | Split: `cache_break.rs` (the comparison), `application/prefix_watch.rs` (the recent-request memory) |
| `src/bin/gateway.rs` | 1063 | `gateway/interface/http/proxy_handler.rs` | Split, see 7.3; `src/bin/gateway.rs` remains as a 30-line root |
| `src/bin/delegate_mcp.rs` | 596 | `orchestration/interface/mcp/mcp_server.rs` | Split: `json_rpc.rs`, `tool_schema.rs`, `tool_arguments.rs`, `lazy_delegator.rs`, `compose.rs`; root remains |
| `src/bin/code.rs` | 405 | `tui/app/interactive_session.rs` | Split: `compose.rs`, `print_mode.rs`, `startup.rs`, `doctor_command.rs`; root remains |
| `src/bin/ask.rs` | 52 | `src/bin/ask.rs` | Logic to `backend/application/ask_once.rs` |
| `src/bin/chat.rs` | 121 | `src/bin/chat.rs` | Loop to `chat/interface/chat_repl.rs` |
| `src/bin/tune.rs` | 106 | `src/bin/tune.rs` | Logic to `calibration/application/sweep_cheap_threshold.rs` |
| `src/bin/route.rs` | 306 | `src/bin/route.rs` | Logic to `calibration/application/dry_run_routes.rs` |
| `src/bin/orch_tune.rs` | 115 | `src/bin/orch_tune.rs` | Logic to `calibration/application/sweep_orchestration_cutoff.rs` |
| `src/cli.rs` | 263 | `tui/cli_args.rs` | Move |
| `src/commands.rs` | 294 | `tui/command.rs` | Move |
| `src/console.rs` | 1234 | `tui/console/console.rs` | Split, see 7.3 |
| `src/lineedit.rs` | 543 | `tui/input/line_editor.rs` | Split: `completion_helper.rs`, `history_file.rs`, `interrupt_guard.rs`, `answer_key.rs`; raw-mode calls to `shared/infrastructure/sys_unix.rs` |
| `src/repl.rs` | 785 | `tui/repl/repl.rs` | Split: `turn_race.rs` (the Ctrl-C race, `SigInt`), `command_handlers.rs`, `pager.rs` |
| `src/views.rs` | 555 | `tui/views/status_facts.rs` | Split one screen per file |
| `src/terminal.rs` | 215 | `tui/tty/terminal_guard.rs` | Split: `terminal_size.rs`, `screen.rs`, `panic_hook.rs`; termios and ioctl calls to `sys_unix.rs` |
| `src/uifmt/mod.rs` | 115 | `tui/render/mod.rs` | Move; its two cross-formatter tests go to `render/tests.rs` |
| `src/uifmt/commands.rs` | 80 | `tui/command_table.rs` | Move (it is the command table, not a formatter) |
| `src/uifmt/complete.rs` | 227 | `tui/render/completion.rs` | Move |
| `src/uifmt/{ansi,changes,failure,markdown,permissions,provider_view,route,screens,status,style,summary,text,width,wrap}.rs` | 3,253 | `tui/render/<same name>.rs` | Move, file for file |
| `src/uifmt/testutil.rs` | 70 | `tui/render/test_facts.rs` | Move |

---

## 5. Type strictness

The user's rule: "safe typing, no any at all, or never, or unknown. All types must be defined.
Strictest codebase ever." In Rust terms, seven policies.

### 5.1 The policies

| # | Policy | Rust meaning |
|---|--------|--------------|
| T1 | No untyped data | `serde_json::Value`, `serde_json::Map` and `json!` do not appear in production code outside the allow-listed wire files of 5.2, and never in any function signature or struct field that another file can see. Every external payload is deserialised at its adapter into a defined struct or enum. |
| T2 | No stringly-typed state | A closed set is an `enum`. An identifier is a newtype (`ModelId`, `SessionId`, `WorkerId`, `PermissionRule`, `ToolName`). `Result<_, String>` is replaced by the context's `Error`. |
| T3 | No panics in production | No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, no slice indexing that can panic, outside `#[cfg(test)]`. |
| T4 | No dynamic typing | No `dyn Any`, no downcast, no `Box<dyn Error>`, no `anyhow::Error` in the library. Closure aliases that act as ports become named traits. |
| T5 | `unsafe` in one file | One audited module with a `// SAFETY:` comment per block; `deny(unsafe_code)` everywhere else. |
| T6 | Lossless numbers | No `as` casts that can truncate, wrap or lose sign; `From`/`TryFrom` instead. Probabilities are `Probability`, times are `UnixMillis`. |
| T7 | Warnings are errors | `-D warnings` for rustc and clippy (pedantic, with a short reviewed allow list) in the gate. |

### 5.2 Where untyped JSON is still allowed, and why

Three situations cannot be fully typed without changing behaviour, because the program must
pass through data whose shape it does not own. Each gets one private wire file. `Value` there is
a private field or local variable; the file's public functions take and return defined types.

| Allow-listed file | Why `Value` is needed | Typed surface the rest of the program sees |
|-------------------|-----------------------|--------------------------------------------|
| `approval/infrastructure/claude_settings_wire.rs` | `.claude/settings.local.json` belongs to Claude Code. Keys we do not know must survive a rewrite, in the same order and form as today. | `PermissionSettings { allow: Vec<PermissionRule>, directories: Vec<PathBuf> }`, `add_allow_rule`, `remove_allow_rule`, `add_directory` |
| `gateway/infrastructure/anthropic_wire.rs` | The gateway forwards requests with fields it does not know (new API parameters) and must only replace `model` and adapt a few fields for Haiku. | `MessagesRequest` view: `model`, `messages: Vec<WireMessage>`, `system`, `tools`, `metadata`; `with_model(ModelId) -> Vec<u8>` |
| `backend/infrastructure/claude/permission_wire.rs` | A tool's input is echoed back to the CLI as `updatedInput`. Today it is re-serialised from a `Value`, which sorts keys. To keep the bytes identical, the echo is canonicalised once at this boundary into `OpaqueJson`. | `ToolInput` (typed, for review and rules) plus `OpaqueJson` (a validated JSON text with a redacting `Debug`, never inspected) |

`gateway/domain/prefix_fingerprint.rs` hashes arbitrary request blocks today (`prefix.rs:39`).
It becomes a hash over the typed `WireMessage` blocks and raw block text, computed in
`anthropic_wire.rs`; the domain sees only `PrefixFingerprint` (hashes and lengths).

The allow list is a `const` in `tests/architecture.rs` (check A6). Adding a fourth file needs a
change to that test, which is a reviewed decision, not a drive-by.

### 5.3 How each payload becomes typed without changing behaviour

`Value` probing is lenient: a missing field is `null`, an unknown event type is ignored. A
derived `Deserialize` is strict by default. To keep behaviour, every wire type follows these
rules, and every conversion is proved by a differential test before the old code is deleted.

- Events are `#[serde(tag = "type")]` enums with an `#[serde(other)] Unknown` variant; every
  field is `Option<T>` or `#[serde(default)]`; unknown fields are ignored (never
  `deny_unknown_fields`).
- Number readers keep today's tolerance (`count(v, key)` in `claude.rs:134`, `codex.rs:145`,
  `gemini.rs:143`, `grok.rs:54` accept what they accept today) through one shared
  `deserialize_with` function in the adapter folder.
- **Differential oracle.** In the commit that introduces a typed parser, the old `Value`
  parser moves into the test module. A test feeds both every fixture line in
  `tests/fixtures/<cli>/`, plus mutated lines (each field removed, each field given the wrong
  type, unknown `type`, truncated JSON), and requires identical `Vec<Event>`. The next commit
  deletes the old parser.
- **Output bytes.** `serde_json` is built without `preserve_order` here, so every `json!`
  object is written with keys in alphabetical order, and `Option::None` is written as `null`.
  A typed row struct therefore declares its fields in alphabetical order and never uses
  `skip_serializing_if` unless the old code omitted the key. `tests/formats.rs` pins one
  golden line per format **before** the conversion (step P0-3), so a byte difference fails.

| Payload | Today (untyped) | Typed replacement |
|---------|-----------------|-------------------|
| Jev request | `state: Value, questions: Value` (`typesafe.rs:58-112`), 8 question builders and 3 state builders returning `Value` (`router.rs:528-693`) | `JudgmentState { request, recent_context, previous: Option<PreviousTurn> }`, `Question { id: QuestionId, kind: Choice(Vec<Criterion>) \| Noul(TrueFalse), instructions }`; serialised by `typesafe_wire.rs` |
| Jev response | `ask* -> Result<Value>`, `decode_choice(&Value, key)`, `decode_noul` (`typesafe.rs:132-138`), `decode_provider_want(&Value, ...)` (`router.rs:704`) | `Answers` (map `QuestionId -> Answer::{Choice(ChoiceAnswer), Noul(Probability)}`) deserialised in `typesafe_wire.rs` |
| Signals for the log | `pub signals: Value` in `CodeWant` (`router.rs:27`) and `ProviderWant` (`router.rs:53`); `Value::Null` for "not asked" (`router.rs:462`); `Judged::log_signals -> Value` (`turn.rs:270`); `jev: &Value` (`turn.rs:1062`, `delegate.rs:1356`, `delegate.rs:1373`) | `Signals` struct in `judgment/domain/signals.rs`; "not asked" is `Option<Signals>`; `SignalsRow` in the log wire files writes the same keys |
| Signals read back by key | `worker_alias(tier: usize, signals: &Value, ...)` reads `signals["edit_p"]`, `["confidence"]`, `["choice"]` (`router.rs:225-235`); `worker_want -> Option<(&'static str, String, Value)>` (`router.rs:264`) | `fn worker_alias(want: &TierWant, ...) -> WorkerAlias` (enum `Haiku \| Sonnet \| Opus`); `worker_want -> Option<WorkerJudgment>` |
| Orchestration fields | `Orchestration::fields -> Value` (`router.rs:121`) | `OrchestrationRow` in `gateway/infrastructure/decision_log_wire.rs` |
| Claude stream | 10 functions over `&Value` (`backend/claude.rs:97-181`) | `ClaudeLine` enum in `claude_wire.rs` |
| Codex stream | 6 functions over `&Value` (`backend/codex.rs:120-182`) | `CodexLine` enum in `codex_wire.rs` |
| Gemini and agy streams | 9 functions over `&Value` (`backend/gemini.rs:143-282`) | `GeminiLine`, `AgyLine` |
| Grok stream | 5 functions over `&Value` (`backend/grok.rs:54-103`) | `GrokLine` |
| Tool input | `PermissionRequest.input: serde_json::Value` (`backend/mod.rs:150`), read by about 20 functions in `permissions.rs:41-550` | `ToolInput` enum: `Bash { command }`, `Write { path, content }`, `Edit { path, old, new }`, `MultiEdit { path, edits }`, `NotebookEdit { .. }`, `WebFetch { url }`, `Other { fields: Vec<ToolField> }` (each unknown field already rendered to text at the adapter) |
| Tool call summary | `Event::Tool(String)` (`backend/mod.rs:123`), parsed back by splitting on whitespace in `handoff.rs:225-250` | `Event::Tool(ToolCall { name: ToolName, argument: String })`; `session::tool_trail` reads the fields; the display string is built by `tui/render` |
| Turn log row | `turn_row(..., signals: &Value, ...) -> Value` (`turnlog.rs:38-46`), `usage_json`, `plan_json` | `TurnLogRow`, `UsageRow`, `PlanRow` (`turn_log_wire.rs`) |
| `jev-ask` output | `ask_json -> Value` (`turnlog.rs:69`) | `AskOutput` (`backend/interface/ask_output.rs`) |
| Log writer | `log::append(path, &Value)` (`log.rs:48`) | `jsonl_log::append<T: Serialize>(path, &T)` |
| Saved session | `Sessions::to_json/from_json(&Value)` (`handoff.rs:359-382`), `SavedSession::to_json/from_json` (`handoff.rs:469-480`), `read_session` (`handoff.rs:572`) | `SavedSessionFile` DTO in `session_wire.rs` with the same leniency as `from_json` today |
| Cooldown file | `Cooldowns::to_json/from_json(&Value)` (`cooldown.rs:314-320`) | `CooldownFile` DTO |
| Chain overrides | `let value: Value = serde_json::from_str(json)` (`catalog.rs:271`) | `ChainsFile` DTO; error text kept |
| Codex auth token | two `Value` parses (`availability.rs:381-383`) | `CodexAuthFile`, `CodexIdClaims` DTOs |
| Worker record | `Saved` built with `json!` (`delegate.rs:676-733`) | `WorkerFile` DTO |
| MCP protocol | 22 uses of `Value` in `bin/delegate_mcp.rs` (`tools_list`, `parse_delegate`, `rpc_result`, `handle`) | `JsonRpcRequest`, `JsonRpcResponse`, `ToolCallParams`, `DelegateArguments`, `ShowDiffArguments` in `json_rpc.rs`, `tool_arguments.rs`. The tool input schemas are static JSON text checked by a test |
| Gateway usage | `UsageTap.record: Value` (`usage.rs:113`), `set(key, Value)` (`usage.rs:147`), `error: Option<Value>` (`usage.rs:106`), SSE handling over `&Value` (`usage.rs:194-213`), `decision: Value` (`usage.rs:335`, `usage.rs:457`), `finish_check(.., Option<Value>)` (`usage.rs:472`) | `UsageRecord`, `SseEvent` enum, `ModelChoiceRecord`, `CheckResult` enum |
| Gateway request reading | 47 uses in `bin/gateway.rs` (`choose_model(.., req: &Value)` line 565, `last_user_prompt` 712, `recent_context` 801, `adapt_for_haiku(&mut Value)` 827, `set_model` 849) | `MessagesRequest` view from `anthropic_wire.rs`; pure rules in `gateway/domain` take `&[WireMessage]` |

### 5.4 Stringly-typed states and loose signatures found

| Where | Today | Becomes |
|-------|-------|---------|
| `router.rs:14-19` | `Decision { model: &'static str, kind: String, reason: &'static str }` | `TierRoute { tier: LegacyTier, kind: RequestKind, reason: TierReason }` |
| `router.rs:23`, `orchestrate.rs:45`, `orchestrate.rs:57`, `router.rs:225` | tier as `usize`, indexing `TIERS` arrays (`bin/gateway.rs:51`, `bin/route.rs:38`, `turn.rs:45`) | `LegacyTier` enum with `model_id()` |
| `anthropic.rs:23` | `Message.role: String` | `Role` enum |
| `backend/mod.rs:106` | `PlanUsage.window: String` | `PlanWindow` enum with `Other(String)` for a window name the CLI invents |
| `backend/mod.rs:203`, `handoff.rs:22`, `turn.rs:237`, `delegate.rs:892` | model as `String` | `ModelId` |
| `backend/mod.rs:86-88` | `ErrorKind::Quota { resets_at: Option<String> }` | keeps the CLI's wording as `ResetText` (newtype); parsed by `providers::reset_time` |
| `turn.rs:49-91` | `Ui::route(&str)`, `note(&str)`, `no_route(.., text: &str)`, `cancelled(text: &str)`: the engine formats text for the screen | `TurnObserver` receives typed facts (`RouteDecision`, `Note` enum, `NoRouteInfo`, `CancelInfo`); `tui/render` writes the words |
| `turn.rs:279-281`, `delegate.rs:978-979` | `BackendFactory`, `GitStatus`, `Clock`, `Judge` as `Box<dyn Fn ...>` aliases | named traits in `ports.rs` |
| 33 signatures | `Result<_, String>` in `delegate.rs` (12), `bin/delegate_mcp.rs` (10), `catalog.rs` (3), `permissions.rs` (3), `bin/code.rs` (2), `backend/mod.rs`, `handoff.rs`, `repl.rs`, `routing.rs` (1 each) | the context's `Error` enum; the text shown to the user is produced by `Display` and pinned by tests |
| `router.rs`, `typesafe.rs`, `delegate.rs`, `anthropic.rs` | `anyhow::Result` in four library files | `judgment::Error`, `orchestration::Error`, `chat::Error` |
| `usage.rs:147`, `usage.rs:472` | record fields set by string key | fields of `UsageRecord` |

### 5.5 Panics, `unsafe`, downcasts found

Production code today has 11 panic sites, `unsafe` in 3 files, and 1 downcast.

| Where | Today | Fix |
|-------|-------|-----|
| `catalog.rs:180` | `panic!("catalog chain names unknown model ...")` | Chains name `&'static ModelSpec` constants instead of ids, so an unknown name is a compile error |
| `turn.rs:1123` | `.expect("legacy tiers are catalog models")` | `LegacyTier::spec()` returns the constant |
| `handoff.rs:541` | `.expect("an unbounded range")` | a `loop` that returns |
| `permissions.rs:666-669` | three `.expect("checked in parse")` on a `Value` | the typed settings view owns the lists |
| `usage.rs:522` | `.expect("checked")` | `if let Some(open) = ...` |
| `prefix.rs:125` | `.lock().unwrap()` | `shared::lock` |
| `bin/gateway.rs:297`, `:607` | `.unwrap()` on a response builder and on a checked `Option` | a fallible builder mapped to a fixed 502 response; `let Some(prompt) = ... else` |
| `bin/route.rs:284` | two `.expect("object")` on `Value` | a typed output row |
| `backend/mod.rs:877`, `:945` | `unsafe` `killpg` and `kill(pid, 0)` | `sys_unix::signal_process_group`, `sys_unix::process_exists` |
| `lineedit.rs:303-352` | `unsafe` `read`, `tcgetattr`, `tcsetattr` (7 blocks) | `sys_unix::RawMode` guard, `sys_unix::read_byte` |
| `terminal.rs:36-136` | `unsafe` termios save and restore, `ioctl(TIOCGWINSZ)` (6 blocks) | `sys_unix::saved_termios`, `restore_termios`, `set_echo`, `window_size` |
| `terminal.rs:169-171` | `payload().downcast_ref::<&str>()` and `::<String>()` | `PanicHookInfo::payload_as_str()` (stable since Rust 1.81; the toolchain here is 1.97), same result for both payload kinds |

`shared/infrastructure/sys_unix.rs` is the only file with `#![allow(unsafe_code)]`. Each block
gets a `// SAFETY:` comment; `clippy::undocumented_unsafe_blocks` is denied. The crate root has
`#![deny(unsafe_code)]` (not `forbid`, because `forbid` cannot be relaxed for that one file);
every `src/bin/*.rs` has `#![forbid(unsafe_code)]`. Check A7 makes the file list explicit.

44 `as` numeric casts exist in production code; each becomes `From`, `TryFrom` with a typed
error, or a named saturating conversion in `UnixMillis`/`TokenUsage`.

### 5.6 Lint configuration

`Cargo.toml` (the workspace root; the Flashboard design already plans a
`[workspace.lints]` table, so the engine opts in with `lints.workspace = true`):

```toml
[workspace.lints.rust]
unsafe_code = "deny"
unreachable_pub = "deny"
missing_debug_implementations = "deny"
unused_must_use = "deny"
trivial_casts = "deny"
trivial_numeric_casts = "deny"
unused_qualifications = "deny"

[workspace.lints.clippy]
pedantic = { level = "deny", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
unreachable = "deny"
todo = "deny"
unimplemented = "deny"
indexing_slicing = "deny"
string_slice = "deny"
as_conversions = "deny"
disallowed_types = "deny"
disallowed_macros = "deny"
dbg_macro = "deny"
print_stdout = "deny"            # allowed by attribute in tui/console and interface output files only
print_stderr = "deny"
exit = "deny"                    # only src/bin/* may exit
mem_forget = "deny"
undocumented_unsafe_blocks = "deny"
allow_attributes_without_reason = "deny"
wildcard_enum_match_arm = "deny" # a new enum variant must be handled everywhere
too_many_lines = "deny"          # threshold 60 in clippy.toml
too_many_arguments = "deny"      # removes the two #[allow]s at usage.rs:378 and delegate.rs:1368
module_name_repetitions = "allow"   # reviewed: `RouteDecision` in `route_decision.rs` is the naming rule
missing_errors_doc = "allow"        # reviewed: the Error enum documents itself
```

`clippy.toml`:

```toml
disallowed-types = [
  { path = "serde_json::Value", reason = "untyped data: define a wire struct (docs/architecture/DDD.md 5.2)" },
  { path = "serde_json::Map",   reason = "untyped data" },
  { path = "anyhow::Error",     reason = "use the context's Error; anyhow only in src/bin" },
]
disallowed-macros = [{ path = "serde_json::json", reason = "untyped data: serialise a wire struct" }]
too-many-lines-threshold = 60
too-many-arguments-threshold = 6
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-indexing-slicing-in-tests = true
```

Test modules that build JSON fixtures carry one attribute on the `mod tests` item:
`#[allow(clippy::disallowed_types, clippy::disallowed_macros, reason = "test fixtures")]`.
The three wire files of 5.2 carry
`#![expect(clippy::disallowed_types, reason = "...")]`; check A6 requires that exactly those
three do. `#![deny(warnings)]` is not written in source (a new compiler release would break old
commits); the gate passes `-D warnings` instead, which has the same effect at every check.

CI: the repository has no CI today. `scripts/check.sh` is the gate; decision U4 is whether to
add a GitHub Actions workflow that runs it on every push.

---

## 6. SOLID in the target structure

| Principle | How the structure provides it |
|-----------|-------------------------------|
| Single responsibility | One concept per file; a README purpose without "and"; size budgets force the split when a second responsibility grows. |
| Open/closed | A new provider is a new folder under `backend/infrastructure/` and `providers/infrastructure/probes/`, a row in `model_table.rs`, and one `ProviderId` variant; `wildcard_enum_match_arm` makes the compiler list every `match` to extend. No existing rule is edited. |
| Liskov | Every backend passes the same contract tests (`backend/infrastructure/contract_tests.rs`: scripted CLI, same assertions for all five). Fakes in `testkit` implement the same ports. |
| Interface segregation | Small ports per use case (`TurnRunner`, `Approver`, `WorkingTree`, `SessionStore`, `CooldownStore`, `JudgmentService`, `Clock`, `Env`). `Engine` today takes 11 builder options and reaches into `Runtime`'s public fields; in the target it holds ports. `Ui` (8 methods, seven for the screen, one for approvals) splits into `TurnObserver` and `Approver`. |
| Dependency inversion | Use cases name traits from `application/ports.rs`; adapters implement them; `src/bin/*` and `compose.rs` are the only places that name a concrete adapter. |

---

## 7. SOLID and DRY audit of the current code

### 7.1 Layering and responsibility violations

| # | Where | Finding | Removed by |
|---|-------|---------|------------|
| V1 | `turn.rs:335-370`, `:371-1120` | `Engine` is routing state, session store, change tracker, orchestration planner, logger, approver and failover loop in one 750-line `impl` with 50 methods. | Split into the files of `turn/application/` (7.3) |
| V2 | `delegate.rs:1-1643` | Worker policy, patch parsing, git worktrees, a persisted record store, process liveness, crash recovery and the delegation use cases in one file. | `orchestration/` layers (7.3) |
| V3 | `backend/mod.rs` | Domain types (`ProviderId`, `Event`, `ErrorKind`), the approval port, error-text heuristics, prompt temp files, and a 450-line child-process runner in one file. Every module that needs `ProviderId` imports the process runner's module. | `shared/domain/provider_id.rs`, `backend/domain`, `backend/infrastructure/process` |
| V4 | `bin/gateway.rs` (1,063 lines), `bin/delegate_mcp.rs` (596), `bin/code.rs` (405) | Business rules and their tests live in binaries, where no other code (Flashboard) can reuse them. `forward` is 145 lines, `choose_model` 93. | `gateway/`, `orchestration/interface/mcp/`, `tui/app/`; binaries become roots |
| V5 | `providers.rs:455-510`, `availability.rs:168-198`, `turn.rs:591-600` | Screen text is produced in domain and engine code. | `tui/render/providers_text.rs` |
| V6 | `uifmt/failure.rs:33`, `uifmt/permissions.rs:145`, `uifmt/mod.rs` (imports `backend`) | The presentation module, documented as "input structs hold plain strings and numbers, never engine types", imports `doctor`, `permissions` and `backend`. | `tui/views` builds facts; `render` takes facts only |
| V7 | `commands.rs:6-8` | A module documented as pure imports `uifmt`, `turn`, `permissions`, `backend`. | `tui/command.rs` depends on `command_table.rs` and context roots |
| V8 | `catalog.rs:252`, `router.rs:239`, `backend/claude.rs:60-64`, `backend/codex.rs:52-53`, `backend/grok.rs:43`, `backend/gemini.rs:114`, `providers.rs:245`, `:384`, `orchestrate.rs:135` | Constructors and pure modules read the environment themselves (hidden input, not testable without process state). | `config.rs` per context, built once from `&dyn Env` |
| V9 | `backend/claude.rs:24-25`, `backend/mod.rs:26` | The backend layer depends on the chat client module (`anthropic`) for limit-text rules, and on `permissions`. | `backend/domain/limit_text.rs`; `approval` is a declared upstream |
| V10 | `providers.rs:181-204` | `Runtime` has eight `pub` fields mutated from `turn.rs`, `delegate.rs`, `repl.rs` and `views.rs`: no invariant can be kept. | `ProviderRegistry` with methods only |
| V11 | `router.rs` | One file holds the legacy three-tier router, the provider judgment, eight question texts, keyword security lists, thresholds and the worker alias rule. | `judgment/` split (7.3) |
| V12 | `handoff.rs` | Provider session ids, the turn log, catch-up rendering, the saved-session format and its file handling in one file. | `session/` split |
| V13 | `doctor.rs:218-259`, `providers.rs:515` | Provider setup knowledge (install hint, login command) is owned by `doctor` but used by `uifmt` and `providers`. | `providers/domain/setup_guide.rs` |
| V14 | `delegate.rs:61` | Orchestration imports the turn engine for an idle timeout and a factory alias, closing a cycle. | `backend::BackendFactory` port; idle timeout in `backend` config |
| V15 | `turn.rs:49-91` | `Ui` mixes progress reporting and tool approval, and takes pre-formatted text. | `TurnObserver` + `Approver`, typed facts |
| V16 | 16 production files over 400 lines; 10 functions over 60 lines (`bin/gateway.rs:302` 145, `bin/route.rs:137` 119, `turn.rs:679` 109, `bin/gateway.rs:565` 93, `bin/chat.rs:37` 85, `usage.rs:379` 76, `delegate.rs:1369` 72, `changes.rs:714` 67, `bin/tune.rs:40` 67, `turn.rs:932` 64) | Size hides structure. | Budgets (9.2) |

### 7.2 Duplication

| # | What is duplicated | Where | One home |
|---|--------------------|-------|----------|
| D1 | Running `git` | `changes.rs:245` (`git`), `changes.rs:302`, `delegate.rs:386` (`git`), `turn.rs:284` (`git_status_short`), `doctor.rs:463` (`in_git_repo`) | `shared/infrastructure/git_command.rs` |
| D2 | Current time in ms | `log.rs:28`, `doctor.rs:543`; clock closure aliases `turn.rs:281`, `delegate.rs:979`; nanosecond stamps for unique names at `changes.rs:268`, `backend/mod.rs:430`, `delegate.rs:1119` | `Clock` port; `private_file::unique_name` |
| D3 | Write a private file atomically | `handoff.rs:546` (`write_atomic`), `permissions.rs:711` (`write_private_atomic`), `backend/mod.rs:425` (`write_private`), `cooldown.rs:343-349`, `delegate.rs:758-764`, mode setting at `lineedit.rs:163` | `shared/infrastructure/private_file.rs` |
| D4 | Parse a float from the environment | `bin/gateway.rs:105` (`env_f64`), `bin/route.rs:132` (`env_or`), `envfile.rs:54` (`lookup_f64`). The first two skip the env file and accept non-finite values, the third does not | `Env::f64(EnvVar, default)`; the difference is kept per variable and written down in `EnvVar` |
| D5 | Idle timeout from the environment | `turn.rs:298-307` (`CODE_IDLE_TIMEOUT_SECS`), `bin/ask.rs:21` (`ASK_IDLE_SECS`) | `backend/infrastructure/config.rs` |
| D6 | Home directory | eight reads of `HOME`: `envfile.rs:43`, `doctor.rs:491`, `providers.rs:106`, `log.rs:14`, `availability.rs:233`, `bin/code.rs:222`, `:271`, `permissions.rs:547` | `Env::home()` |
| D7 | Lock a mutex ignoring poison | `usage.rs:368`, `providers.rs:282`, `backend/codex.rs:70`, `console.rs:130`, `backend/gemini.rs:102`, `delegate.rs:990`, `backend/mod.rs:489`, `:496`, `turn.rs:95` | `shared/domain/lock.rs` |
| D8 | Cut text to a length | `handoff.rs:298` (`truncate_chars`), `handoff.rs:308` (`truncate_lines`), `delegate.rs:339` (`truncate_with_note`), `backend/mod.rs:585` (`last_chars`) | `shared/domain/truncation.rs` (four named functions, since their outputs differ) |
| D9 | "How long" in words | `providers.rs:165` (`format_left`), `views.rs:186` (`human_ago`), `uifmt/text.rs` | `shared/domain/duration_text.rs` |
| D10 | Legacy tier table | `bin/gateway.rs:51`, `bin/route.rs:38`, `turn.rs:45`, plus `router.rs:9` and `routing.rs:18` importing the constants | `catalog/domain/legacy_tier.rs` |
| D11 | "Is this a harness message" | `bin/gateway.rs:664-680` (two functions), `bin/orch_tune.rs:31` (one function with all four prefixes, shorter spellings) | `gateway/domain/harness_message.rs`, both tools call it with the same options they use today |
| D12 | `MINUTE_MS` and time constants | `providers.rs:31`, `cooldown.rs:18-21` | `shared/domain/unix_millis.rs` |
| D13 | Is this file executable | `changes.rs:487`, `availability.rs:215` | `shared/infrastructure/private_file.rs` neighbour `file_mode` function in `sys_unix.rs` |
| D14 | Usable models list | `availability.rs:156` (`usable_models`), `views.rs:61` (`usable_models`) | `providers::ProviderRegistry::usable` |
| D15 | Per-CLI JSON number reader | `backend/claude.rs:134` (`count`), `backend/codex.rs:145` (`count`), `backend/gemini.rs:143` (`uint`), `backend/grok.rs:54` (`count`); four `usage_of` functions | one `deserialize_with` in `backend/infrastructure/` |
| D16 | Test scaffolding | temp dir helpers at `cooldown.rs:763`, `lineedit.rs:486`, `views.rs:474`, `permissions.rs:849`, `console.rs:971`, `delegate.rs:1880`, `backend/mod.rs:1369`, `changes.rs:826`; fake script backends at `turn.rs:1157`, `repl.rs:433`, `delegate.rs:1825`, `backend/mod.rs:975`, `:1600`; temp git repos at `changes.rs:859`, `repl.rs:500`, `turn.rs:2103`, `delegate.rs:2161` | `src/testkit/` |
| D17 | Tool summary formatted then parsed | written as `"Name args"` by each backend, split again in `handoff.rs:225` | `ToolCall` (5.3) |
| D18 | Attempt logging | `turn.rs:1062` (`log_attempt`), `delegate.rs:1332` (`log_attempt`), both building a `turn_row` | `turn_log.rs` writer taking a typed `AttemptLog`, used by both through a port |

### 7.3 How the big files split

Line ranges are today's; each destination file is within the 400-line budget.

**`turn.rs` (2,743: 1,125 production, 1,618 tests)**

| Today | Destination |
|-------|-------------|
| `Ui` trait 49-91 | `turn/application/ports.rs` (`TurnObserver`) |
| `EngineApprover` 100-141 | `turn/application/approve_tool.rs` |
| `Failover`, `NoRouteInfo` 143-168 | `turn/domain/failover.rs`, `no_route_info.rs` |
| `Cancel` 170-213 | `turn/application/cancel.rs` |
| `TurnStatus`, `TurnReport` 216-252 | `turn/domain/turn_report.rs` |
| `Judged` 254-277 | `turn/domain/judged.rs` |
| aliases, `git_status_short`, idle timeout 279-307 | `ports.rs`, `infrastructure/git_status.rs`, `backend` config |
| `Chosen`, `Attempt` 310-333 | `turn/domain/attempt.rs` |
| `Engine` struct and builder 335-462 | `engine.rs`, `engine_builder.rs` |
| accessors, `new_session`, save, resume 464-590 | `session_commands.rs` |
| `last_changes`, `last_diff`, `undo_last` 509-536 | `change_commands.rs` |
| `run*`, `judge` 606-662 | `run_turn.rs` |
| `run_attempts` 679-790 | `run_attempts.rs` (the loop, with the body of one iteration as its own function) |
| `choose`, `legacy_choice` 791-832, 885-920 | `choose_model.rs` |
| `plan_orchestration`, `apply_orchestration`, `mcp_providers` 833-884 | `plan_orchestration.rs` |
| `prompt_for`, `run_attempt` 921-998 | `run_attempt.rs` |
| `finish`, `cancelled_report`, `record_failed`, `no_route_message` 999-1120 | `finish_turn.rs` |
| `log_attempt` 1062-1086 | `infrastructure/turn_log.rs` |
| tests 1126-2743 | `turn/application/tests/`: `fakes.rs`, `routing.rs`, `failover.rs`, `handoff.rs`, `approval.rs`, `orchestration.rs`, `cancel.rs`, `changes.rs`, `persistence.rs` |

**`delegate.rs` (2,856: 1,643 production)**

| Today | Destination |
|-------|-------------|
| `BriefSignals`, `worker_task`, `worker_floor` 85-131 | `orchestration/domain/brief_signals.rs` |
| `NoWorker`, `WorkerView`, `candidate_lists`, `spread`, `pick_worker` 132-216 | `domain/worker_pick.rs` |
| patch parsing and paging 218-347 | `domain/file_patch.rs`, `patch_page.rs` |
| `worker_prompt` 349-365 | `domain/worker_prompt.rs` |
| `Worktrees` trait 367-385 | `application/ports.rs` |
| `git`, `Snapshot`, `explain_apply_failure`, `checked_inside`, `GitWorktrees` 386-573 | `infrastructure/git_worktrees.rs`, `worktree_base.rs` (the second `Snapshot`, renamed) |
| `Worker`, `Owner` 575-640 | `domain/worker.rs` |
| `Liveness`, `ProcLiveness` 641-675 | `ports.rs`, `infrastructure/process_liveness.rs` |
| `Saved`, `Records`, `Store` 676-790 | `infrastructure/worker_file.rs`, `worker_wire.rs`; trait to `ports.rs` |
| `Found`, `Fate`, `Scan` 791-878 | `domain/recovery.rs` |
| `DelegateRequest`, `DelegateResult` 880-962 | `domain/delegation.rs`, `delegation_result.rs` |
| `DelegateApi`, `Judge` 963-988 | `ports.rs` |
| `CreateSlot`, `Reservation` 1000-1083 | `application/reservation.rs` |
| `Delegator` struct and builder 1084-1277 | `application/delegator.rs` |
| `recover` 1278-1318 | `application/recover_workers.rs` |
| `run_worker`, `attempt`, `run_attempt` 1319-1442 | `application/run_worker.rs` |
| `delegate` 1443-1476 | `application/delegate_work.rs` |
| `show_diff` 1477-1539 | `application/show_worker_diff.rs` |
| `merge`, `discard` 1540-1603 | `application/merge_worker.rs`, `discard_worker.rs` |

**`backend/mod.rs` (1,874: 964 production)**

| Today | Destination |
|-------|-------------|
| `ProviderId` 31-76 | `shared/domain/provider_id.rs` |
| `Usage` 77-85 | `shared/domain/token_usage.rs` |
| `ErrorKind`, `PlanUsage`, `Finish`, `Event` 86-139 | `backend/domain/error_kind.rs`, `plan_usage.rs`, `event.rs` |
| `PermissionRequest`, `Decision`, `Approver`, `NoApprover` 142-199 | `approval/domain/permission_request.rs`, `decision.rs`; `Approver` to `backend/application/ports.rs` |
| `TurnRequest`, `TurnOutcome`, `TurnError` 201-240 | `backend/domain/` |
| `McpAttachment` 242-251 | `backend/domain/mcp_attachment.rs` |
| `Backend` trait 253-300 | `backend/infrastructure/cli_dialect.rs` (it returns a `tokio::process::Command`, so it is an adapter contract); the application port is `TurnRunner` |
| text classification 303-387 | `backend/domain/error_text.rs` |
| `PromptArg`, prompt files 389-507 | `domain/prompt_arg.rs`, `infrastructure/process/prompt_files.rs` |
| `StdinFeed`, stderr capture, `Pump` 509-775 | `process/stdin_feed.rs`, `stderr_tail.rs`, `output_pump.rs`, `idle_watchdog.rs` |
| `judge`, `reap`, `explain_eof`, `run_turn`, `run_turn_with`, `run_child` 776-942 | `process/child.rs`, `cli_turn_runner.rs` |
| `GroupKill`, `process_alive`, `is_zombie` 843-963 | `process/process_group.rs` over `sys_unix` |

**`changes.rs` (1,586: 820 production)**: types 31-117 to `domain/file_change.rs`,
`snapshot.rs`, `change_set.rs`; git output parsing 119-183 and 660-670 to
`infrastructure/git_output.rs`; `merge_untracked` 185-243 to `domain/untracked_merge.rs`;
temp index 266-325 to `infrastructure/git_temp_index.rs`; `snapshot`, `summarize`, `diff_text`
327-422 to `infrastructure/git_working_tree.rs` behind the `WorkingTree` port; new-file diff
424-497 to `domain/new_file_diff.rs`; the cancellable wrappers 502-520 to
`application/track_turn.rs`; undo planning 577-658 to `domain/undo_plan.rs`, `safe_path.rs`;
the 67-line `undo` 714-783 to `application/undo_turn.rs` in three named steps (plan, verify
nothing changed since, apply); file restoring 785-820 to `infrastructure/undo_files.rs`.

**`permissions.rs` (1,348: 840 production)**: review text 30-240 to `approval/domain/review.rs`
and `visible_text.rs`; rule policy 244-500 to `rule_policy.rs`, `bash_rule.rs`,
`permission_rule.rs`; `Always` 508-560 to `always_plan.rs`; `validate_rule` 562-586 to
`permission_rule.rs`; the settings document 588-760 to
`infrastructure/claude_settings_file.rs` and `claude_settings_wire.rs`; `resolve_dir` 763-778
to `infrastructure/directory_path.rs`; `Listing` 780-816 to `domain/listing.rs` and
`application/list_rules.rs`; `/permissions` argument parsing 818-840 to
`tui/permissions_command.rs` (it is command-line syntax, not approval policy).

**`console.rs` (1,234: 606 production)**: `Console`, `ConsoleConfig`, `Inner` output
serialisation to `tui/console/console.rs`; `Ticker` and the live status line to
`status_ticker.rs`; `impl Ui for Console` (391-473) to `turn_observer.rs`; the approval
question, key reading and "always" handling (501-568) to `approval_prompt.rs`; `Buf` test
writer to `testkit`.

**`router.rs` (1,197: 832 production)**: `Decision`, `Sticky`, the tier mapping (14-200) to
`routing/domain/legacy_tier_route.rs`; `ProviderWant` to `judgment/domain/judgment.rs`,
`CodeWant` to `tier_want.rs`; thresholds 57-151 to `thresholds.rs`; `Orchestration` 88-136 to
`orchestration_judgment.rs`; `Router` 202-526 to `application/judge.rs`, with `worker_want`
and `worker_alias` to `judge_worker.rs` and `answer_is_adequate` to `verify_answer.rs`; the
question texts 528-693 to `domain/question_catalog.rs`; decoding 704-737 to
`infrastructure/typesafe_wire.rs`; keyword lists and `security_level` 739-828 to
`security_keywords.rs`; `is_short_follow_up` 829 to `short_follow_up.rs`.

**`bin/gateway.rs` (1,063: 881 production)**: `App`, `Orch`, `main` to
`gateway/interface/http/compose.rs` and `routes.rs`; `orchestrate_check`, `worker_check` to
`application/check_orchestration.rs`, `check_worker.rs` with thin handlers in
`check_handlers.rs`; `forward` (145 lines) to `application/forward_request.rs` as four steps
(read request, choose model, send with Haiku fallback, tap response); `send_upstream`,
`upstream_request` to `infrastructure/anthropic_upstream_http.rs`; `choose_model` (93 lines)
to `application/choose_model.rs` over pure rules in `domain/model_choice.rs`; prompt text
functions 664-825 to `domain/prompt_text.rs`, `harness_message.rs`, `recent_context.rs`;
`adapt_for_haiku`, `without_long_context` to `domain/haiku_adaptation.rs`; `set_model` to
`infrastructure/anthropic_wire.rs`.

---

## 8. Migration plan

### 8.1 Ground rules

1. One step is one commit. After every commit `scripts/check.sh` passes. A commit either moves
   code or changes code, never both.
2. Files move with `git mv`, in a commit that changes nothing but `mod` lines and `use` paths,
   so git records a rename with near-100% similarity and `git log --follow` keeps working. A
   split comes in later commits: the original file keeps the largest part (it carries the
   history), the extracted parts are new files (`git blame -C -C` still finds their origin).
3. Old import paths keep working during the migration through alias modules in `lib.rs`
   (`pub mod router { pub use crate::judgment::*; }`). They are in the architecture baseline
   and are removed in the last phase. This is what lets workers split different contexts at
   the same time without editing each other's files.
4. **Ownership is by folder.** In a parallel phase one worker owns one context folder and may
   not edit any file outside it. `lib.rs`, `Cargo.toml`, `clippy.toml`, `scripts/`,
   `tests/architecture*` and `src/shared/` are edited only in serial steps.
5. No reformatting. The code is not `rustfmt`-default formatted today (138 lines of `turn.rs`
   alone exceed 100 columns), and a reformat inside a move would destroy rename detection.
   Formatting is decision U5 and, if wanted, is one separate commit at the very end.
6. Nothing observable changes: binary names, flags, every environment variable, file names
   and formats under the state directory, log row bytes, CLI argument lists, exit codes, screen
   text. `tests/formats.rs`, `scripts/test_pty_ux.py` and `scripts/parity.sh` check this.
7. Freeze: all open work (`fix-orch-trigger`, review fixes) merges before P1 starts, because P1
   moves every file. Flashboard starts after P5 or imports only `jev_router::embed`.
8. No step runs `grok`, `agy`, `gemini`, `claude` or `codex`, and none calls Jev. All tests use
   the existing fakes.

### 8.2 Steps

Risk: L = mechanical, compiler-checked; M = structural, covered by existing tests; H = changes
how data is parsed or written, needs the oracle or golden tests named in the step.

**Phase 0: safety net (serial, one worker, 7 steps)**

| Step | What | Risk | Verification beyond the gate |
|------|------|------|------------------------------|
| P0-1 | Add `scripts/check.sh` with today's checks only (build, clippy at the current level, tests, PTY, Python). Record the baseline: warnings and clippy findings today. | L | The script passes on the unchanged tree |
| P0-2 | Add the `fixture!` macro; replace the 24 relative `include_str!` paths (`backend/*.rs`, `permissions.rs:1145`). | L | Same tests pass |
| P0-3 | Add `tests/formats.rs`: one golden byte string for each format: turn log row, `jev-ask` line, gateway request/turn/decision/check records, delegate log row, `cooldowns.json`, saved session file, worker record, MCP config file, `settings.local.json` after each edit, the Jev request body, Claude `control_response` lines, and each backend's argument list. | L | Goldens generated from the current code and reviewed by reading |
| P0-4 | Add `tests/architecture.rs` with checks A1-A12 and `tests/architecture_baseline.txt` listing every current violation. | L | The test passes with the baseline; deleting a baseline line makes it fail |
| P0-5 | Add `scripts/budget.py` in report mode (lists offenders, exits 0) and `scripts/parity.sh` (builds the tagged base and HEAD, runs both `jev-code --help`, `jev-code doctor` in a hermetic HOME with fake CLIs, `jev-delegate-mcp` `initialize` and `tools/list`, `jev-ask` against a fake CLI, and diffs the outputs). | L | Parity of the tree against itself is empty |
| P0-6 | Create `src/testkit/` and move the duplicated test scaffolding of D16 into it, one helper kind per commit (4 commits: scratch dir, temp git repo, scripted backend, capture writer). | M | Test count unchanged (764) |
| P0-7 | Tag `ddd-base`. | L | none |

**Phase 1: whole-file moves (serial, one worker, 15 steps, scripted)**

Each step creates one context folder with `README.md` and `mod.rs`, `git mv`s that context's
files to the first destination named in 4.5, fixes `mod`/`use` lines, and adds the alias module
in `lib.rs`. No file content changes otherwise. Order is upstream first.

| Step | Context | Files moved | Risk |
|------|---------|-------------|------|
| P1-1 | `shared` | `envfile.rs`, `log.rs` | L |
| P1-2 | `catalog` | `catalog.rs` | L |
| P1-3 | `judgment` | `typesafe.rs`, `router.rs` | L |
| P1-4 | `routing` | `routing.rs` | L |
| P1-5 | `approval` | `permissions.rs` | L |
| P1-6 | `backend` | `backend/*` (5 files) | L |
| P1-7 | `providers` | `providers.rs`, `availability.rs`, `cooldown.rs` | L |
| P1-8 | `session` | `handoff.rs` | L |
| P1-9 | `changes` | `changes.rs` | L |
| P1-10 | `orchestration` | `orchestrate.rs`, `delegate.rs` | L |
| P1-11 | `turn` | `turn.rs`, `turnlog.rs` | L |
| P1-12 | `doctor` | `doctor.rs` | L |
| P1-13 | `gateway`, `chat` | `usage.rs`, `prefix.rs`, `anthropic.rs`, `transcript.rs` | L |
| P1-14 | `tui` | `uifmt/*` (17 files), `console.rs`, `lineedit.rs`, `repl.rs`, `views.rs`, `terminal.rs`, `commands.rs`, `cli.rs` | L |
| P1-15 | binaries | Move the bodies of the 8 binaries into `interface/` files as whole files (`git mv src/bin/gateway.rs src/gateway/interface/http/proxy_handler.rs`, then a new 30-line `src/bin/gateway.rs`), including `calibration`. | M (binary tests move with the file; `parity.sh` must be empty) |

After P1: tag `ddd-p1`. Verification for every P1 step adds `git diff -M --stat HEAD~1`
showing only renames plus `lib.rs`/`mod.rs` lines.

**Phase 2: split inside each context (parallel, up to 14 workers, one lane per context or small group)**

Each lane is a sequence of extraction commits following 7.3 and 4.5: move items to a new file
in the right layer folder, re-export them from the context `mod.rs` under the same names, move
their tests with them. No signature changes. A lane removes its own lines from the
architecture baseline in its last commit, which is the only file outside its folder it touches;
those baseline commits are rebased in order by the integrator.

| Lane | Context | Extraction commits (about) | Risk | Depends on |
|------|---------|---------------------------:|------|-----------|
| P2-a | `shared` | 6 | L | P1 |
| P2-b | `catalog` | 8 | L | P1 |
| P2-c | `judgment` | 10 | M | P1 |
| P2-d | `routing` | 8 | L | P1 |
| P2-e | `approval` | 10 | M | P1 |
| P2-f | `backend` | 22 | M (the process runner is timing-sensitive; its tests at `backend/mod.rs:965-1874` run unchanged) | P1 |
| P2-g | `providers` | 20 | M | P1 |
| P2-h | `session` | 9 | L | P1 |
| P2-i | `changes` | 12 | M | P1 |
| P2-j | `orchestration` | 30 | M | P1 |
| P2-k | `turn` | 20 | M | P1 |
| P2-l | `doctor`, `chat`, `calibration` | 14 | L | P1 |
| P2-m | `gateway` | 22 | M | P1 |
| P2-n | `tui` | 25 | M (PTY test is the judge) | P1 |

Items that change context during the split (for example `ProviderId` from `backend` to
`shared`, `PermissionRequest` from `backend` to `approval`, backend construction from
`providers` to `backend`, setup hints from `doctor` to `providers`, `render*` to `tui`) are a
serial sub-phase **P2-x** (8 commits, one worker) run before the lanes start, because each
touches two contexts. The old path stays as a re-export.

After P2: tag `ddd-p2`. Every file is in its final folder; budgets are met for files; names
are unchanged.

**Phase 3: seams and duplication (introduce serially, adopt in parallel, remove serially)**

| Step | What | Mode | Risk |
|------|------|------|------|
| P3-1 | `shared`: `Env` port, `EnvVar` enum, `Secret`; `Clock`, `StateDir` ports; `private_file`, `git_command`, `lock`, `truncation`, `duration_text`, `unix_millis`. The old free functions stay and delegate to the new code. | serial | M |
| P3-2 | Each context adds `config.rs` and takes configuration and clock through constructors; replaces its copies of D1-D15 with the shared ones. One lane per context (14 lanes, 2-6 commits each). | parallel | M (D4: keep each variable's exact parsing; a test per variable) |
| P3-3 | Remove the old free functions (`envfile::lookup`, `log::now_ms`, `log::state_dir`) once no caller remains; A8 leaves the baseline. | serial | L |
| P3-4 | Ports: `TurnRunner` and `BackendFactory` in `backend`; `WorkingTree` in `changes`; `SessionStore` in `session`; `CooldownStore` in `providers`; `JudgmentService` in `judgment`; `Worktrees`/`Records`/`Liveness` moved to `ports.rs`. Adapters implement them; closure aliases removed. One lane per context. | parallel | M |
| P3-5 | `turn`: `Ui` becomes `TurnObserver` + `Approver` with typed facts; `tui/console/turn_observer.rs` produces the same text. `ProviderRegistry` hides its fields. | serial (touches `turn`, `tui`, `orchestration`) | H (screen text: PTY test plus `parity.sh`) |
| P3-6 | Remove cycles V14, V6, V7; A1-A5 leave the baseline. | serial | M |

After P3: tag `ddd-p3`.

**Phase 4: type strictness (shared types first, then parallel lanes)**

| Step | What | Mode | Risk |
|------|------|------|------|
| P4-1 | `shared`: `ModelId`, `Probability`, `UnixMillis`, `TokenUsage`, `OpaqueJson`; `jsonl_log::append<T: Serialize>` beside the old one. | serial | L |
| P4-2 | `backend`: typed wire enums for claude, codex, gemini, agy, grok, each with the differential oracle of 5.3 (one commit adds the typed parser and oracle, the next deletes the old parser). `ToolCall`, `ToolInput`. | lane | H |
| P4-3 | `judgment`: typed `Question`, `Answers`, `Signals`; golden Jev request body unchanged. | lane | H |
| P4-4 | `turn` + `orchestration`: `TurnLogRow`, `AttemptLog`, typed delegate log; `signals: Value` removed. | lane (after P4-3) | H (golden rows) |
| P4-5 | `session`, `providers`, `catalog`: file DTOs (`session_wire`, `cooldown_wire`, `chains_wire`, `codex_auth_wire`). Old files written by the current release must still load: a test loads a file captured before the change. | 3 lanes | H |
| P4-6 | `approval`: `ToolInput` in review and rule policy; settings wire file. | lane (after P4-2) | H (rule policy is a security boundary: every existing rule test runs unchanged) |
| P4-7 | `orchestration/interface/mcp`: typed JSON-RPC. | lane | M |
| P4-8 | `gateway`: `MessagesRequest` view, `UsageRecord`, `SseEvent`; `Value` confined to `anthropic_wire.rs`. A test forwards a recorded request and requires the upstream body to be byte-identical to today's. | lane | H |
| P4-9 | Typed errors: one `Error` per context; `Result<_, String>` and `anyhow` removed from the library. User-visible error text is pinned by tests first. | lanes | M |
| P4-10 | Newtypes and enums of 5.4 (`LegacyTier`, `Role`, `PlanWindow`, `WorkerId`, `PermissionRule`, ...). | lanes | M |
| P4-11 | Remove the 11 panic sites and 44 `as` casts; move `unsafe` to `sys_unix.rs`; replace the downcast. | lanes, then serial for `sys_unix` | M |
| P4-12 | Turn on the lint tables of 5.6 one group at a time (`unsafe_code`, `unwrap_used` group, `disallowed_*`, `pedantic`), each commit fixing what the group reports. A6, A7 leave the baseline. | serial | M |

After P4: tag `ddd-p4`.

**Phase 5: closing (serial, 7 steps)**

| Step | What | Risk |
|------|------|------|
| P5-1 | Remove the alias modules from `lib.rs`; rewrite remaining old paths (scripted). | L |
| P5-2 | Rename to the glossary: `Runtime` to `ProviderRegistry`, `ChangeSummary` to `ChangeSet`, `ProviderWant` to `Judgment`, `Sessions` to `Session`, the second `Gate`/`Snapshot`/`Decision`/`SessionInfo`/`Interrupt`. One rename per commit. `embed.rs` keeps `Runtime` as an alias. | L |
| P5-3 | Function budget: split the functions still over 60 lines. | M |
| P5-4 | `src/embed.rs` and `tests/embed_api.rs`. | L |
| P5-5 | Write every context `README.md` from the template (9.3); `//!` owns/must-not on every module; A10-A12 leave the baseline. | L |
| P5-6 | `budget.py` to failing mode; delete `tests/architecture_baseline.txt` (it is empty); update the "Layout" section of `README.md` (lines 476-500) and `scripts/provider_eval_README.md:23`. | L |
| P5-7 | Run `scripts/install.sh` into a temporary destination; run `parity.sh` against `ddd-base`; tag `ddd-done`. | L |

**Count.** 7 + 15 + (1 serial sub-phase + 14 lanes) + 6 + 12 + 7 = **62 planned steps**, about
330 commits. Serial: P0, P1, P2-x, P3-1, P3-3, P3-5, P3-6, P4-1, P4-12, P5. Parallel: the 14
lanes of P2, the per-context lanes of P3-2 and P3-4, and the lanes of P4-2 to P4-11.

### 8.3 Parallel work without conflicts

| Phase | Workers | Each worker owns | Shared files, and who edits them |
|-------|--------:|------------------|----------------------------------|
| P0, P1 | 1 | everything | n/a |
| P2 | up to 14 | one context folder | `tests/architecture_baseline.txt`: each lane's last commit, applied in order by the integrator |
| P3-2, P3-4 | up to 14 | one context folder | `src/bin/*` and `compose.rs` wiring: the integrator, after each lane lands |
| P4-2 to P4-11 | up to 8 | one context folder | `tests/formats.rs` is split per context (`tests/formats/<context>.rs`) in P0-3 so lanes do not collide |
| P5 | 1 | everything | n/a |

Each worker uses its own git worktree and branch (`ddd/<phase>-<context>`); the integrator
rebases lanes onto `refactor/ddd` one at a time and runs the gate after each.

### 8.4 Verification

| After | Run |
|-------|-----|
| every commit | `scripts/check.sh` (3.5): warning-free build, clippy, 764+ Rust tests including architecture and formats, PTY UX test, three Python test files |
| every move commit | `git diff -M --stat HEAD~1`: renames only |
| every phase | `scripts/parity.sh ddd-base`: empty diff; test count not lower than before; `python3 -I scripts/budget.py` offender count not higher than before |
| P4 lanes | the differential oracle and golden tests named in the step |
| the end | `scripts/install.sh "$tmp"` then `"$tmp/jev-code" --help`, `"$tmp/jev-delegate-mcp"` `initialize`; `git log --follow` on five moved files reaches their pre-refactor history |

Review loop: after each phase a reviewer reads the phase diff; findings are fixed and the
review repeats until a round finds nothing, as for all work on this project.

### 8.5 Rollback

- All work is on `refactor/ddd`; `master` and the installed binaries are untouched until the
  final merge.
- Every commit is green, so any single step is undone with `git revert <sha>`: a move commit
  reverts to the old location, an extraction reverts to the unsplit file.
- Phase tags (`ddd-base`, `ddd-p1` ... `ddd-p4`, `ddd-done`) are safe stopping points: each is a
  working, shippable tree. If the work must stop, the branch is merged at the last tag; the
  ratchet baseline documents exactly what remains.
- A P4 step that turns out to change bytes (a golden or oracle test cannot be made to pass
  without changing behaviour) is not forced: the payload is added to the 5.2 allow list with a
  written reason, or the step is dropped, and the user is told.
- After the final merge, a regression in the field is handled by reinstalling from the
  `ddd-base` tag (`git checkout ddd-base && scripts/install.sh`); state files and logs are
  format-compatible in both directions because no format changed.

---

## 9. Readability rules

### 9.1 The one-minute test

A person who has read only the root `README.md` section "Layout" (16 lines, one per context)
and one context `README.md` can name the file that holds a given behaviour. The definition of
done includes ten such questions with their expected answers, for example:

| Question | Answer |
|----------|--------|
| Where is it decided that a correction keeps the stronger model? | `routing/domain/switch_gate.rs` |
| Where is the wording of the Jev task-type question? | `judgment/domain/question_catalog.rs` |
| Where does a Codex output line become an Event? | `backend/infrastructure/codex/codex_wire.rs` |
| Where is "Bash(cargo test:*)" derived from a command? | `approval/domain/bash_rule.rs` |
| Where is the length of a cooldown computed? | `providers/domain/cooldown_length.rs` |
| Where is the catch-up shortened to fit its budget? | `session/domain/catch_up.rs` |
| Where does `/undo` refuse because a file changed since? | `changes/domain/undo_plan.rs` |
| Where is a worker model picked? | `orchestration/domain/worker_pick.rs` |
| Where does the gateway swap `jev-code` for a real model? | `gateway/application/choose_model.rs` |
| Where is the terminal put back after a panic? | `tui/tty/panic_hook.rs` |

### 9.2 Budgets, enforced by `scripts/budget.py`

| Budget | Limit | Today |
|--------|------:|-------|
| Lines per `.rs` file, including inline tests | 400 | 16 files exceed it on production code alone; the largest is 2,856 |
| Lines per test-only file (`*_tests.rs`, `tests/*.rs`) | 600 | n/a |
| Lines per function (also `clippy::too_many_lines`) | 60 | 10 functions exceed it; the largest is 145 |
| Function parameters | 6 | 2 `#[allow(clippy::too_many_arguments)]` |
| `pub use` names in a context `mod.rs` | 25 | n/a |
| Files in one folder (`src/` itself holds the 16 contexts, `lib.rs`, `embed.rs`, `testkit/`, `bin/`) | 20 | `src/` has 30 modules side by side |
| `src/bin/*.rs` lines | 30 | up to 1,063 |

The script prints every offender with its size and exits non-zero when one exists (report mode
until P5-6). It has no exceptions list: a file that cannot meet the budget is two concepts.

### 9.3 Rules of form

1. One concept per file, named after the concept in the glossary.
2. The same role has the same file name in every context (`ports.rs`, `error.rs`, `config.rs`,
   `*_wire.rs`, `*_file.rs`, `*_http.rs`, `*_cli.rs`, `git_*.rs`, `compose.rs`).
3. No grab-bag names (check A12).
4. Every module starts with `//!` in two sentences: "Owns ...", "Must not ...".
5. A context's purpose sentence has no "and" joining two responsibilities; neither does a
   module's "Owns" sentence. Reviewers check this; `budget.py` flags `" and "` in the first
   line of a `README.md` as a warning.
6. The public surface of a context is the `pub use` list of its `mod.rs`, repeated in its
   README. Nothing else is importable.
7. Every context README follows one template:

```
# <context>
<One sentence: the purpose.>

## Language        the glossary terms this context owns, one line each
## Public surface  the names exported by mod.rs, grouped: types, use cases, ports, adapters
## Layout          one line per file: what it owns
## Depends on      the contexts it may import, and what it takes from each
## Must not        the things this context never does (for example: "never reads the clock")
## Where is ...    five questions a newcomer asks, each answered with a file name
```

### 9.4 Before and after: the turn context

**Before.** One file, `src/turn.rs`, 2,743 lines. To learn how a failed attempt moves to the
next provider you open it, scroll past a UI trait, an approver, a cancel token, report types,
three closure aliases and an 11-option builder, and find a 109-line `run_attempts` that calls
ten private methods spread over the next 400 lines, three of which are about orchestration and
two about logging. Its tests start at line 1,126 and run for 1,618 lines in one module.

```
src/turn.rs                                          2,743 lines
  Ui (8 methods: progress + approval, takes formatted text)      49
  EngineApprover                                                100
  Failover, NoRouteInfo, Cancel                                 143
  TurnStatus, TurnReport, Judged                                216
  BackendFactory / GitStatus / Clock closure aliases            279
  git_status_short (spawns git), idle_timeout_from_env          284
  Engine { 21 fields }                                          335
  impl Engine { 50 methods }                                    371-1120
     builder, accessors, sessions, undo, save, resume,
     run, judge, snapshot, run_attempts, choose, orchestration,
     legacy choice, prompt, run_attempt, finish, logging, messages
  mod tests { everything }                                      1126-2743
```

**After.** The folder reads as the steps of a turn. Each file fits on a few screens.

```
src/turn/
  README.md               "Carrying one turn from prompt to report."
  mod.rs                  pub use: Engine, EngineBuilder, TurnObserver, TurnReport, TurnStatus,
                                   Judged, Failover, NoRouteInfo, Cancel, Error
  error.rs
  domain/
    turn_report.rs        TurnReport, TurnStatus          what a turn ends as
    judged.rs             Judged                          what Jev said, or that it was not asked
    attempt.rs            Attempt, MAX_ATTEMPTS           one try on one model
    failover.rs           Failover                        why a turn moved on
    no_route_info.rs      NoRouteInfo                     why nothing could take it
  application/
    ports.rs              TurnObserver, TurnLog, GitStatus
    engine.rs             Engine: the state one session keeps (ports + session + routing state)
    engine_builder.rs     EngineBuilder
    run_turn.rs           1. judge the prompt, snapshot the tree, run the attempts, report
    run_attempts.rs       2. the failover loop: choose, run, react, try the next
    choose_model.rs       3. ask routing for the one model (or the legacy tier)
    plan_orchestration.rs 4. give the turn a planner when the gate opens
    run_attempt.rs        5. catch the model up, run it, forward events
    approve_tool.rs       6. answer a tool approval through the observer
    finish_turn.rs        7. record the turn, log it, build the report
    cancel.rs             Cancel
    session_commands.rs   /new, /resume, save
    change_commands.rs    /diff, /undo
    tests/                fakes.rs  routing.rs  failover.rs  handoff.rs  approval.rs
                          orchestration.rs  cancel.rs  changes.rs  persistence.rs
  infrastructure/
    turn_log.rs           TurnLog over the JSON-lines log
    turn_log_wire.rs      TurnLogRow (the row's exact shape)
    git_status.rs         GitStatus over git
    config.rs             TurnConfig from the environment
```

The same question, "how does a failed attempt move to the next provider?", is answered by
opening `turn/application/run_attempts.rs`. "What does a turn log row contain?" is
`turn/infrastructure/turn_log_wire.rs`. "Which test covers failover after a quota error?" is
`turn/application/tests/failover.rs`.

---

## 10. Definition of done

Every line is a command or a file check.

**Readability**
- [ ] `python3 -I scripts/budget.py` exits 0: no `.rs` file over 400 lines (test-only files
      600), no function over 60 lines, no `src/bin/*.rs` over 30 lines, no folder over 20
      files, no context `mod.rs` exporting more than 25 names.
- [ ] Every context folder has a `README.md` with the seven template sections (check A11).
- [ ] Every module starts with a `//!` "Owns / Must not" comment (check A11).
- [ ] No file or module is named `util`, `utils`, `helpers`, `misc`, `common`, `types`,
      `manager` (check A12).
- [ ] Every `mod.rs` contains only docs, `mod` items and explicit `pub use` lists (check A10).
- [ ] The ten questions of 9.1 are answered correctly from the tree and READMEs alone by
      someone who did not do the refactor (or a fresh agent with no other context), each in
      under a minute.
- [ ] Each overloaded name of 1.1 (`Decision`, `Gate`, `Snapshot`, `SessionInfo`, `Usage`,
      `Interrupt`, `run_turn`) has one definition: `grep` finds one `struct|enum|trait|fn`.

**Architecture**
- [ ] `tests/architecture_baseline.txt` does not exist and `cargo test --test architecture`
      passes (checks A1-A12).
- [ ] `src/lib.rs` declares exactly the 16 contexts, `embed` and `testkit`.
- [ ] `tests/embed_api.rs` passes; `src/embed.rs` contains only `pub use` lines.
- [ ] Each duplication D1-D18 has one definition (a `grep` per row finds one).

**Type strictness**
- [ ] `cargo clippy --all-targets -- -D warnings` passes with the lint tables of 5.6.
- [ ] `serde_json::Value`, `serde_json::Map` and `json!` appear in production code only in the
      three files of 5.2 (check A6).
- [ ] `unsafe` appears only in `src/shared/infrastructure/sys_unix.rs`, every block has a
      `// SAFETY:` comment, and every `src/bin/*.rs` has `#![forbid(unsafe_code)]` (check A7).
- [ ] No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `as` cast, `dyn Any`, downcast,
      `anyhow` or `Result<_, String>` in `src/` outside `#[cfg(test)]` and `src/bin/`.
- [ ] `std::env` is read only in `src/shared/infrastructure/env.rs` (check A8);
      `Secret::expose` is called only in `infrastructure/` (check A9).

**No behaviour change**
- [ ] `scripts/check.sh` passes: warning-free build, all Rust tests (at least the 764 that
      exist today), `scripts/test_pty_ux.py`, the three Python test files.
- [ ] `cargo test --test formats` passes with goldens unchanged since P0-3.
- [ ] `scripts/parity.sh ddd-base` prints no difference.
- [ ] `scripts/install.sh "$tmp"` installs `jev-code`, `jev-ask`, `jev-delegate-mcp`,
      `jev-chat`, `jev-gateway`, `jev-claude`; `scripts/install.sh` itself is unchanged
      (`git diff ddd-base -- scripts/install.sh` is empty).
- [ ] `Cargo.toml` has the same eight `[[bin]]` names and no new dependency
      (`git diff ddd-base -- Cargo.lock` shows no added package).
- [ ] `git log --follow --oneline` on `src/turn/application/engine.rs`,
      `src/orchestration/application/delegator.rs`,
      `src/backend/infrastructure/process/child.rs`,
      `src/changes/infrastructure/git_working_tree.rs` and
      `src/approval/domain/rule_policy.rs` shows commits older than `ddd-base`.
- [ ] A final review round over the whole diff finds nothing.

---

## 11. Decisions for the user

| # | Decision | Recommendation | Consequence of the other choice |
|---|----------|----------------|---------------------------------|
| U1 | One crate with a test-enforced rule, or one crate per context | One crate now (3.2); revisit after done | Per-context crates give compiler enforcement and `forbid(unsafe_code)` in 15 of 16 crates, at roughly double the migration risk and a rewrite of the Flashboard `embed` plan |
| U2 | Scope of this refactor: structure only (phases 0-3 and 5), or structure plus type strictness (phase 4) | Both, in this order; phase 4 is where the "strictest" requirement is met | Stopping after phase 3 gives the readable tree and the layering, but `Value` stays in about 300 places |
| U3 | The three allow-listed wire files of 5.2 | Accept them: each exists only to pass through data the program does not own, byte for byte | Zero `Value` anywhere is possible only by changing bytes sent to Claude Code or written to its settings file, which breaks "no behaviour change" |
| U4 | CI | Add a GitHub Actions workflow that runs `scripts/check.sh` on push | Without it the gate is run by hand and by the agents only |
| U5 | Formatting | Leave formatting alone during the refactor; decide afterwards whether to adopt `rustfmt` with a `max_width` matching today's style, as one final commit | Formatting during the moves destroys `git log --follow` |
| U6 | Glossary renames in P5-2 (`Runtime` to `ProviderRegistry`, `ChangeSummary` to `ChangeSet`, `ProviderWant` to `Judgment`) | Do them; they are compiler-checked and the names appear in no file format | Skipping them keeps three vague names in an otherwise consistent vocabulary |
| U7 | Budgets | 400 lines per file, 60 per function, as in 9.2 | Looser numbers leave `run_attempts`-sized functions in place; tighter ones (300/40) add about 40 more files |
| U8 | Timing | Merge all open branches first, then freeze feature work until `ddd-p1` (about one day of serial moves); Flashboard starts on `embed` after `ddd-done` | Feature work during P1 conflicts with every moved file |
