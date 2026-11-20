# jev-router: target architecture (DDD) and migration plan

Status: proposal. Scope: the Rust crate `jev-router` at tag `ddd-start` on `refactor/ddd`,
before merging to `main`. `find src -name '*.rs'` finds 173 files with 71,093 lines.
Counting from the first `#[cfg(test)]` in each file as test code gives 42,557 production
lines and 28,536 test lines. The task reports 1,543 passing library tests; this count has
not been verified in this worktree. This document moves no code.

The migration audit is not yet fully pinned to `ddd-start`. Unless updated below, file
locations, line numbers, counts, the file table and the import graph refer to commit
`296bd5b`. They must be checked before relying on them. Step P0-0 completes that audit.

The refactor is **pure**: no behaviour change, no change to binary names, flags, environment
variables, on-disk state, log rows or what any CLI receives on its command line or stdin.

## 0. Goals, in priority order

| #   | Goal                     | What it means here                                                                                                                                                                                     | How it is checked                                                                              |
| --- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| 1   | **Readability**          | From the folder tree and one README, a newcomer finds where any behaviour lives in under a minute. One concept per file, small files, the same file names for the same roles in every context.         | Section 9 (budgets, naming, README test), enforced by `scripts/check.sh`                       |
| 2   | **Strict typing**        | No untyped data inside the program. Every external payload becomes a defined struct or enum at the adapter that reads it. No `unwrap`, no stringly-typed states, no `unsafe` outside one audited file. | Section 5, enforced by lints and the architecture test                                         |
| 3   | **DDD, SOLID, DRY**      | One crate per bounded context, with a domain that does no IO, use cases that depend on ports, adapters behind those ports. Each duplication found in section 7 has exactly one home.                   | Section 3: Cargo and the compiler hold the context map, the architecture test holds the layers |
| 4   | **Safety of the change** | Every commit builds without warnings and passes every test. History survives (`git mv`). Any step can be reverted alone.                                                                               | Section 8                                                                                      |

The user's decisions are recorded in section 11 (U1: one crate per context; U3: three
allow-listed wire files; U4: CI). Everything else is decided here.

Notation: `<context>/<layer>/<file>.rs` is short for `crates/<context>/src/<layer>/<file>.rs`.
A bare `name.rs:123` is a line of today's flat `src/`.

---

## 1. Domain analysis

### 1.1 Ubiquitous language

One word, one meaning, everywhere: in type names, file names, log fields and screens. The
"today" column shows where the current code uses a different or overloaded name.

| Term                   | Meaning                                                                                                                                                     | Today                                                                             |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| **Turn**               | One user prompt handled end to end: judge, route, catch up, run, fail over, record.                                                                         | `turn::Engine::run*`                                                              |
| **Attempt**            | One try of a turn on one model. A turn has up to `MAX_ATTEMPTS` (6).                                                                                        | `turn::Attempt`                                                                   |
| **Provider**           | A vendor account the user can use: Claude, DeepSeek, Codex, Gemini, Grok. Closed set.                                                                       | `backend::ProviderId`                                                             |
| **Backend**            | The adapter that runs one attempt on one provider's CLI and turns its output into Events.                                                                   | `backend::Backend`                                                                |
| **Model**              | One model of one provider, identified by a `ModelId`.                                                                                                       | bare `String` / `&'static str`                                                    |
| **ModelSpec**          | Catalog entry for a model: provider, id, tier, billing, context size, task limits.                                                                          | `catalog::ModelSpec`                                                              |
| **Tier**               | Capability class of a model: Light, Standard, Strong, Top.                                                                                                  | `catalog::Tier`; also a bare `usize` index in `router.rs:23`, `orchestrate.rs:45` |
| **LegacyTier**         | The older three-step ladder Haiku/Sonnet/Opus (plus Fable) used by the gateway and by routing-off mode.                                                     | `usize` + three copies of a `TIERS` array                                         |
| **TaskType**           | What kind of work a turn is (security, hard_debug, review, feature, simple_edit, explain, ...). Jev decides it.                                             | `catalog::TaskType`                                                               |
| **Judgment**           | Jev's typed answer about one turn: task type, its confidence, and probabilities (edit, security, very hard, continues, correction, multi-part, sequential). | `router::ProviderWant`, `router::CodeWant`, and `signals: serde_json::Value`      |
| **Signals**            | The numbers of a Judgment, as routing reads them and as the log records them.                                                                               | `routing::Signals` plus an untyped `Value` copy                                   |
| **Question**           | One typed thing asked of Jev: a Choice (pick one of named options) or a Noul (a probability).                                                               | `serde_json::Value` built with `json!`                                            |
| **Chain**              | The ordered list of models that may answer a TaskType, before availability is applied.                                                                      | `catalog::default_chain`, `chain_with`                                            |
| **Overrides**          | The user's replacement chains (`JEV_CHAINS`).                                                                                                               | `catalog::Overrides`                                                              |
| **Availability**       | What a startup probe learned about a provider: Ready, NotInstalled, NotLoggedIn, Unknown, with detail.                                                      | `availability::Availability`, `State`                                             |
| **Cooldown**           | A provider is skipped until its usage limit resets. Persisted, shared between processes.                                                                    | `cooldown::Cooldowns`                                                             |
| **Block**              | A provider is skipped for this session for a reason that is not a limit (login, unusable directory).                                                        | `providers::BlockReason`                                                          |
| **PlanUsage**          | How full a provider's plan window is, as the CLI last reported it.                                                                                          | `backend::PlanUsage`                                                              |
| **Allowances**         | What the user permits beyond subscriptions (paid credits, Fable).                                                                                           | `catalog::Allowances`                                                             |
| **Usable**             | The set of models that can be used right now: available, not cooling, not blocked, not refused.                                                             | `catalog::Usable`                                                                 |
| **ProviderRegistry**   | Everything known about providers while the program runs. Answers "who is usable now".                                                                       | `providers::Runtime` (vague name)                                                 |
| **Pin**                | The user's override of routing: auto, a provider, or one model.                                                                                             | `routing::Pin`                                                                    |
| **RouteDecision**      | The one model chosen for an attempt, with the reason in plain words.                                                                                        | `routing::RouteDecision`                                                          |
| **NoRoute**            | No usable model may answer this turn, and why.                                                                                                              | `routing::NoRoute`                                                                |
| **Hold / Switch gate** | The rule that keeps the current model unless Jev is confident a switch is needed.                                                                           | `routing::switch_gate`, `hold_within`                                             |
| **Failover**           | An attempt failed in a way another model can cover, so the turn moves on.                                                                                   | `turn::Failover`                                                                  |
| **Event**              | One thing a Backend saw in the CLI's stream: tool call, text, finished, failed, plan usage, permission request.                                             | `backend::Event`                                                                  |
| **ToolCall**           | A tool the model ran: name and argument.                                                                                                                    | a formatted `String` (`"Bash cargo test"`) parsed back in `handoff.rs:225`        |
| **Session**            | The dialogue of one `jev-code` run: its turn records and each provider CLI's own session id.                                                                | `handoff::Sessions`                                                               |
| **TurnRecord**         | What one finished turn left behind: prompt, reply, who answered, tool calls.                                                                                | `handoff::TurnRecord`                                                             |
| **Handoff**            | Sending a turn to a provider that missed earlier turns.                                                                                                     | `handoff`                                                                         |
| **CatchUp**            | The plain-text block that brings that provider up to date. Built without any model call.                                                                    | `handoff::Sessions::catch_up`, `render`                                           |
| **SavedSession**       | A session written to disk so `/resume` can continue it.                                                                                                     | `handoff::SavedSession`                                                           |
| **Approval**           | Asking the person whether a tool call may run.                                                                                                              | `backend::PermissionRequest`, `Decision`, `Approver`                              |
| **PermissionRule**     | An allow rule in Claude Code's settings file (`Bash(cargo test:*)`).                                                                                        | `String` in `permissions.rs`                                                      |
| **Review**             | What the person is shown about a tool call before deciding.                                                                                                 | `permissions::Review`                                                             |
| **Snapshot**           | The working tree at one instant (a git tree object plus an untracked-file manifest).                                                                        | `changes::Snapshot`; `delegate.rs:417` reuses the name for something else         |
| **ChangeSet**          | What one turn changed: files, counts, and the two Snapshots.                                                                                                | `changes::ChangeSummary`                                                          |
| **Undo**               | Putting the working tree back to the Snapshot before the turn.                                                                                              | `changes::undo`                                                                   |
| **Orchestration**      | A Planner model splits a task and hands pieces to cheaper Workers.                                                                                          | `orchestrate` + `delegate`                                                        |
| **Planner**            | The model that plans and reviews.                                                                                                                           | `orchestrate::choose_planner`                                                     |
| **Delegation**         | One piece of work handed to a Worker: brief, files, read-only flag.                                                                                         | `delegate::DelegateRequest`                                                       |
| **Worker**             | A model doing one Delegation in its own Worktree.                                                                                                           | `delegate::Worker`                                                                |
| **Worktree**           | A git worktree created for one Worker.                                                                                                                      | `delegate::Worktrees`                                                             |
| **OrchestrationGate**  | Whether a turn gets a Planner (multi-part score, sequential veto, tier).                                                                                    | `orchestrate::Gate`; `routing.rs:208` has another `Gate`                          |
| **UsageRecord**        | One gateway request: tokens, latency, status, tools.                                                                                                        | `usage::UsageTap` builds a `Value`                                                |
| **PrefixFingerprint**  | Hashes of a request's prefix, used to say where a prompt cache broke.                                                                                       | `prefix::Fp`                                                                      |
| **Check**              | One line of the `doctor` report: severity, detail, fix steps.                                                                                               | `doctor::Check`                                                                   |

Names that are overloaded today and get exactly one owner: `Decision` (`router.rs:14` and
`backend/mod.rs:164`), `Gate` (`orchestrate.rs:34`, `routing.rs:208`), `Snapshot` (`changes.rs:70`,
`delegate.rs:417`), `SessionInfo` (`handoff.rs:436`, `usage.rs:318`), `Usage` (`anthropic.rs:47`,
`backend/mod.rs:77`), `Interrupt` (`repl.rs:30` trait, `lineedit.rs:35` enum), `run_turn`
(`backend/mod.rs:818`, `repl.rs:83`).

### 1.2 Bounded contexts

Sixteen crates: thirteen domain contexts, the shared kernel, the calibration
tools and the terminal front end (plus `embed`, `testkit` and `cli`, which hold no domain). The purpose sentence is the first line of that
context's `README.md` and contains no "and" joining two responsibilities.

| Context         | Purpose (README first line)                             | Aggregates and entities                                           | Value objects                                                                                                  | Domain services                                                        |
| --------------- | ------------------------------------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `shared`        | The few types and adapters every context needs.         | none                                                              | `ProviderId`, `ModelId`, `Probability`, `UnixMillis`, `TokenUsage`, `Secret`, `OpaqueJson`                     | text truncation, duration wording                                      |
| `catalog`       | Which models exist and which may answer a kind of task. | none (static table)                                               | `ModelSpec`, `Tier`, `Billing`, `TaskType`, `LegacyTier`, `Chain`, `Overrides`, `Usable`, `Allowances`, `Pick` | `chain_for`, `pick`                                                    |
| `judgment`      | Asking Jev typed questions about a turn.                | none                                                              | `Question`, `Answer`, `Judgment`, `Signals`, `OrchestrationJudgment`, `SecurityLevel`                          | `Judge` (use case), keyword security level, short follow-up rule       |
| `routing`       | Choosing the one model for an attempt.                  | `RoutingState`                                                    | `Pin`, `RouteInput`, `RouteDecision`, `NoRoute`                                                                | `decide`, switch gate, plan steering, legacy tier route                |
| `backend`       | Running one attempt on one provider's CLI.              | none                                                              | `TurnRequest`, `TurnOutcome`, `TurnError`, `ErrorKind`, `Event`, `ToolCall`, `PlanUsage`, `McpAttachment`      | error text classification                                              |
| `providers`     | Knowing which providers are usable right now.           | `ProviderRegistry` (root) with `Cooldowns`, blocks, plan readings | `Availability`, `AvailabilityState`, `BlockReason`, `ResetTime`                                                | per-provider `decide_*` from probe facts, cooldown length, setup guide |
| `session`       | Remembering the dialogue across providers.              | `Session` (root) with `TurnRecord`s                               | `CatchUp`, `SavedSession`, `SessionInfo`, `ProjectKey`                                                         | catch-up rendering within a token budget                               |
| `changes`       | Knowing what a turn changed in the working tree.        | `ChangeSet`                                                       | `Snapshot`, `Stamp`, `FileChange`, `ChangeStatus`, `UndoPlan`, `UndoReport`                                    | undo planning, new-file diff                                           |
| `approval`      | Deciding whether a tool call may run.                   | `PermissionSettings`                                              | `PermissionRequest`, `ToolInput`, `Decision`, `Review`, `PermissionRule`, `Refusal`, `AlwaysPlan`              | rule policy, visible-text rendering                                    |
| `orchestration` | Splitting a turn between a planner and workers.         | `Worker` (root, persisted)                                        | `Delegation`, `DelegationResult`, `BriefSignals`, `FilePatch`, `OrchestrationGate`, `PlannerChoice`            | worker pick, patch paging, recovery fate                               |
| `turn`          | Carrying one turn from prompt to report.                | `Engine` (application service holding the session's state)        | `TurnReport`, `TurnStatus`, `Judged`, `Attempt`, `Failover`, `Cancel`                                          | none (it coordinates the other contexts)                               |
| `doctor`        | Telling the user what is not set up.                    | `Report`                                                          | `Check`, `Severity`, `Inputs`                                                                                  | `diagnose`, first-run hint                                             |
| `gateway`       | Serving Claude Code through an Anthropic-format proxy.  | `GatewayTurn` (usage roll-up per user turn), `SessionState`       | `UsageRecord`, `ModelChoice`, `PrefixFingerprint`                                                              | prompt text rules, Haiku adaptation, similarity                        |
| `chat`          | The plain question-and-answer router (`jev-chat`).      | `Transcript`                                                      | `Message`, `Role`                                                                                              | none                                                                   |
| `calibration`   | Reading thresholds off labeled or logged prompts.       | none                                                              | `Scored`, `Cutoff`, `PromptItem`                                                                               | threshold sweep                                                        |
| `tui`           | The terminal front end of `jev-code`.                   | none (interface only)                                             | screen facts (`RouteFacts`, `StatusFacts`, ...)                                                                | formatters                                                             |

`tui` is not a domain context: it is the delivery layer for `turn`, and it renders facts from
several contexts. It has no `domain/`.

### 1.3 Context map

The current import graph still needs to be derived for `ddd-start`. The examples below are
from `296bd5b`, except for the extension trust dependency listed after them.

The levels below are the whole picture: lower levels are upstream. A context may import only
the contexts listed in its row of the table. The table is each crate's `[dependencies]`, so
Cargo refuses a cycle; check C1 (section 3.4) holds the manifests to the table.

```
level 0   shared
level 1   catalog        approval       changes
level 2   judgment       backend
level 3   routing        providers      session
level 4   orchestration  doctor         chat
level 5   turn           gateway
level 6   tui            calibration
level 7   embed (facade for Flashboard)          cli (the eight binaries)

A crate depends only on lower levels, and only on the crates in its row of the table.
testkit sits beside shared and is a dev-dependency only.
```

| Downstream      | Upstream (allowed imports)                                                                                                                                                                                        | Relationship                                                                                                                                                                                                                                                                  |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `catalog`       | `shared`                                                                                                                                                                                                          | Shared kernel                                                                                                                                                                                                                                                                 |
| `judgment`      | `catalog`, `shared`                                                                                                                                                                                               | Conformist to `TaskType`; **Jev is an external service behind the `JudgmentService` port**, and `infrastructure/typesafe_http.rs` is its anti-corruption layer                                                                                                                |
| `routing`       | `judgment`, `catalog`, `shared`                                                                                                                                                                                   | Customer of `judgment` (reads `Signals`)                                                                                                                                                                                                                                      |
| `approval`      | `shared`                                                                                                                                                                                                          | **ACL to Claude Code's settings file** (`infrastructure/claude_settings_file.rs`)                                                                                                                                                                                             |
| `backend`       | `approval`, `shared`                                                                                                                                                                                              | **One ACL per provider CLI** (`claude`, `codex`, `gemini`/`agy`, `grok`); DeepSeek reuses the Claude ACL with different environment                                                                                                                                           |
| `providers`     | `backend`, `catalog`, `shared`                                                                                                                                                                                    | Customer of `backend` (`ErrorKind`, `PlanUsage`); ACLs to each CLI's login probe                                                                                                                                                                                              |
| `session`       | `backend`, `shared`                                                                                                                                                                                               | Published language: `TurnRecord`; reads `backend`'s `ToolCall` value                                                                                                                                                                                                          |
| `changes`       | `shared`                                                                                                                                                                                                          | **ACL to git**                                                                                                                                                                                                                                                                |
| `orchestration` | `backend`, `providers`, `catalog`, `judgment`, `shared`                                                                                                                                                           | Customer; ACL to git worktrees; open-host service to planners through the MCP server                                                                                                                                                                                          |
| `turn`          | every context of levels 0-4 except `doctor` and `chat`                                                                                                                                                            | The core use case; only `tui` and `embed` depend on it                                                                                                                                                                                                                        |
| `doctor`        | `providers`, `judgment`, `shared`                                                                                                                                                                                 | Customer                                                                                                                                                                                                                                                                      |
| `gateway`       | `orchestration`, `routing`, `judgment`, `catalog`, `shared`                                                                                                                                                       | ACL to the Anthropic HTTP API. It reads `orchestration`'s gate (`orch_gate`, `Gate`: `bin/gateway.rs:41`, `:213`) and `routing`'s hold rule (`Sticky`: `bin/gateway.rs:42`); both are domain names                                                                            |
| `chat`          | `judgment`, `catalog`, `shared`                                                                                                                                                                                   | Customer                                                                                                                                                                                                                                                                      |
| `calibration`   | `gateway`, `routing`, `judgment`, `catalog`, `shared`                                                                                                                                                             | Customer (read-only tools). `bin/route.rs` imports `routing`, `router`, `catalog` and `ProviderId` (which moves to `shared`); `bin/orch_tune.rs` reads the gateway's log and shares its harness-message rule (D11). Nothing in the three tools names `providers` or `backend` |
| `tui`           | `turn`, `doctor`, `providers`, `backend`, `approval`, `changes`, `session`, `routing`, `judgment`, `catalog`, `orchestration`, `shared` (what `views.rs`, `repl.rs`, `console.rs` and `bin/code.rs` import today) | Interface                                                                                                                                                                                                                                                                     |
| `embed`         | `turn`, `providers`, `backend`, `approval`, `routing`, `changes`, `session`, `shared`                                                                                                                             | Facade: re-exports only. Until P5-4 it lists `providers` and `shared` only (the six contract names of 4.4)                                                                                                                                                                    |
| `cli`           | `tui`, `gateway`, `orchestration`, `backend`, `chat`, `calibration`, `shared`                                                                                                                                     | Composition roots                                                                                                                                                                                                                                                             |

Each context is a crate (decision U1), so this table is each crate's `[dependencies]` list and
Cargo refuses a cycle. Test code follows the same table: a crate's tests use only that crate's
dependencies plus `jev-testkit`. A test that needs a downstream context lives in that
downstream crate (today one does: `doctor.rs:943` asserts on `uifmt`; it moves to `tui`).

Cycles and upward imports that exist today and disappear: `turn -> orchestrate -> delegate ->
turn` (`delegate.rs:61` imports `turn::{idle_timeout_from_env, BackendFactory}`, `delegate.rs:62`
imports `turnlog::turn_row`); `turn -> uifmt` (`turn.rs:38`, the engine formats screen text);
`uifmt -> doctor` (`uifmt/failure.rs:33` calls `doctor::login_instruction`; the way back,
`doctor.rs:943`, is in a test only); `commands -> uifmt -> backend` for a module documented as
pure.

At `ddd-start`, `backend/claude.rs` also reads `ext::Places` and
`ext::hooks::claude_settings_trusted` while constructing a command. Extension management has
no exact home in 1.2; the closest home for this trust decision is `approval`. P1a-13 moves the
decision out of the backend command builder. The rest of the graph remains to be checked.

**Placements that keep the map acyclic.** Each of these was checked against the call graph;
the obvious home would have closed a cycle or reached into another context's adapter.

| Item today                                                                                                                                   | Why the obvious home fails                                                                                                                       | Home                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `router::Decision` (returned by `Router::route`, `router.rs:315`)                                                                            | In `routing` it would make `judgment -> routing` while `routing -> judgment` exists (`routing.rs:23`)                                            | `judgment/domain/tier_want.rs`, renamed `TierRoute`, beside `CodeWant`                                                                                                                                                                                                                                                                                                                                    |
| `router::Sticky` (consumes `CodeWant`, used by `turn` and `gateway`)                                                                         | It is a hold rule, not a judgment                                                                                                                | `routing/domain/legacy_tier_route.rs`                                                                                                                                                                                                                                                                                                                                                                     |
| `availability::installed_for_real`, PATH and mise checks                                                                                     | Backend construction calls it (`providers.rs:110`); once construction moves to `backend`, leaving it in `providers` makes `backend -> providers` | `backend/infrastructure/path_lookup.rs`, `mise_install.rs`; `providers` probes import them                                                                                                                                                                                                                                                                                                                |
| `gemini::is_configured`, `AUTH_ENV_VARS`, `grok::session_dir_exists`, `codex::allowed_models_from_cache` (used by `availability.rs:418-562`) | They are each CLI's own knowledge, needed by the probes                                                                                          | Stay in the CLI's folder in `backend/infrastructure/`, exported by `backend/mod.rs`; `providers/infrastructure/probes` imports them (rule 4 allows adapter-to-adapter along the map)                                                                                                                                                                                                                      |
| `orchestrate::write_mcp_config` (calls `backend::claude::mcp_config_json`, `orchestrate.rs:173`)                                             | The file format is Claude Code's                                                                                                                 | `backend/infrastructure/claude/mcp_config.rs` owns format and write, behind the `McpConfigWriter` port in `backend/application/ports.rs`; `turn` calls the port                                                                                                                                                                                                                                           |
| `turnlog::turn_row` (shared by `turn.rs:1081` and `delegate.rs:1332`), `usage_json`, `error_kind_name` (also used by `ask_json`)             | `orchestration` may not import `turn`                                                                                                            | All of `turnlog.rs` describes a backend outcome and imports only `backend` and `log`, so the file moves to `backend/infrastructure/outcome_wire.rs`. In P4 the shared parts become `UsageRow`, `PlanRow`, `error_kind_name` there; `SignalsRow` to `judgment/infrastructure/signals_wire.rs`; each writer keeps its own row struct (`turn_log_wire.rs`, `delegate_log_wire.rs`, `ask_output.rs`), see D18 |

---

## 2. Layers inside a context

Every context has the same four folders. A folder is absent when it would be empty (`routing`
is all domain). A layer with a single file is still a folder: the path always names the layer,
which is what both a reader and the architecture test rely on. Nothing is nested deeper than
`crates/<context>/src/<layer>/<group>/<file>.rs`, and a group folder exists only where one layer holds
several adapters of the same kind (`backend/infrastructure/claude/`, `providers/.../probes/`).

Four crates are not layered: `tui` is one delivery layer (every file in it counts as
`interface`), `testkit` is test code, `embed` holds re-exports only, and `cli` holds the eight
binaries.

| Layer          | Folder            | Holds                                                                                           | May do                                                     |
| -------------- | ----------------- | ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Domain         | `domain/`         | Entities, value objects, rules, domain services.                                                | Pure computation. Time and randomness arrive as arguments. |
| Application    | `application/`    | Use cases, and `ports.rs` with the traits those use cases need from the outside.                | Call the domain, call ports, be `async`.                   |
| Infrastructure | `infrastructure/` | Adapters that implement ports: CLIs, HTTP, files, git, the clock. Wire and file formats (DTOs). | Any IO.                                                    |
| Interface      | `interface/`      | Delivery: HTTP handlers, the MCP server, one-shot command output.                               | Parse input, call a use case, format output.               |

Binaries in `crates/cli/src/bin/` are the composition roots: they read configuration, build adapters, hand
them to use cases, and contain nothing else (30 lines or fewer each).

---

## 3. The dependency rule and its enforcement

### 3.1 The rule

| #   | Rule                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Enforced by                                                                                                  |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| 1   | **Inward only.** Inside a crate: `interface -> application, domain` and `infrastructure -> application, domain`. `domain` imports nothing from the other three. `application` never imports `infrastructure` or `interface`. `interface` and `infrastructure` do not import each other, with two stated exceptions: a `compose.rs` wires adapters, and an interface file may import the row structs of its own crate's `*_wire.rs` files (a response is a format: `backend/interface/ask_output.rs` builds its line from `outcome_wire.rs`). The crate-root `error.rs` counts as `domain`. | Architecture test (A1)                                                                                       |
| 2   | **Through the front door.** Another context is imported only as `jev_<context>::Name`. Its layer modules are private.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | The compiler                                                                                                 |
| 3   | **Along the map.** A crate depends only on the crates in its row of 1.3.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Cargo (`[dependencies]`; a cycle does not build), plus check C1 that the lists equal the table               |
| 4   | **Layer-preserving across contexts.** A `domain` file imports from another crate only names that originate in that crate's `domain`. An `application` file imports its `domain` and `application` names, so a use case never names another context's adapter. An `infrastructure` or `interface` file may import any exported name: adapters build on upstream adapters (`shared`'s `git_command`, `private_file`, `jsonl_log`; `backend`'s CLI facts used by the `providers` probes). Files named `compose.rs` and the binaries wire concrete adapters and are exempt.                    | Architecture test (A4)                                                                                       |
| 5   | **Outside crates by layer**, table below.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Cargo for whole crates (a crate that does not list `tokio` cannot use it), architecture test (A5) for layers |

| Layer            | Allowed                                                                                                                                                                                                                                                                                           | Forbidden                                                                                                                                                                                                                                       |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain`         | `std` except the modules on the right                                                                                                                                                                                                                                                             | `std::fs`, `std::process`, `std::env`, `std::net`, `std::io` (other than `std::io::Error` as a field type), `SystemTime::now`, `Instant::now`, `tokio`, `reqwest`, `axum`, `rustyline`, `libc`, `serde`, `serde_json`, `anyhow`, `futures_util` |
| `application`    | `std` (same exclusions, but `Instant::now` is allowed for timeouts), `tokio::sync`, `tokio::time`, `tokio::task`, `tokio::spawn`, `tokio::select!`, `std::future` (the delegator and the cancellable snapshot run blocking work with `spawn_blocking` today: `delegate.rs`, `changes.rs:502-520`) | `tokio::process`, `tokio::fs`, `tokio::net`, `reqwest`, `axum`, `rustyline`, `libc`, `serde`, `serde_json`, `anyhow`                                                                                                                            |
| `infrastructure` | everything the crate's `Cargo.toml` lists                                                                                                                                                                                                                                                         | `anyhow`                                                                                                                                                                                                                                        |
| `interface`      | `axum` (gateway), `rustyline` (tui), `serde`/`serde_json` for request and response DTOs                                                                                                                                                                                                           | `reqwest`, `libc`, `anyhow`                                                                                                                                                                                                                     |
| `cli/src/bin/*`  | `anyhow` allowed here only                                                                                                                                                                                                                                                                        | logic                                                                                                                                                                                                                                           |

`serde` is forbidden in `domain` on purpose: a file or log format must not change because a
domain field was renamed. Formats are DTO structs in `infrastructure` with `From`
conversions, and golden tests pin their bytes.

Where each outside crate is used today, which fixes which `Cargo.toml` may list it:

| Outside crate                  | Used today in                                  | Crates that list it                                                 |
| ------------------------------ | ---------------------------------------------- | ------------------------------------------------------------------- |
| `axum`                         | `bin/gateway.rs`                               | `gateway`                                                           |
| `reqwest`                      | `typesafe.rs`, `usage.rs`, `bin/gateway.rs`    | `judgment`, `gateway`                                               |
| `rustyline`                    | `lineedit.rs`                                  | `tui`                                                               |
| `unicode-width`                | `uifmt/width.rs`                               | `tui`                                                               |
| `libc`                         | `backend/mod.rs`, `lineedit.rs`, `terminal.rs` | `shared` only (all three call sites move to `sys_unix.rs`)          |
| `futures-util`                 | `bin/gateway.rs`, `repl.rs`                    | `gateway`, `tui`                                                    |
| `anyhow`                       | 4 library files, 6 binaries                    | `cli` only                                                          |
| `tokio`, `serde`, `serde_json` | most modules                                   | each crate that needs them; `routing` lists no outside crate at all |

### 3.2 One crate per context (decision U1)

The workspace has 19 members: the 16 contexts of 1.2 as library crates, plus `embed`,
`testkit` and `cli`. Folder `crates/<context>/`, package `jev-<context>`, library
`jev_<context>`. The package in `crates/cli/` keeps the name `jev-router`.

| Concern      | What changes                                                                                                                                                                                                                                    | How it is handled                                                                                                                                                                                                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Boundaries   | The compiler and Cargo enforce the context map and the front door.                                                                                                                                                                              | Rules 2 and 3 need no test. The architecture test shrinks to layer direction inside a crate and the rules of form (3.4).                                                                                                                                                            |
| Binaries     | The eight `[[bin]]` targets move from the root package to `crates/cli/Cargo.toml` with the same names. A workspace has one `target/` at the root, so `target/release/code`, `target/debug/code`, `target/release/ask` are where they are today. | Tried in a scratch workspace: `cargo build --release --bin code --bin ask` at a virtual workspace root builds both into `target/release/`. `scripts/install.sh`, `scripts/test_pty_ux.py` (`target/debug/code`) and `scripts/provider_eval.py` (`target/release/ask`) need no edit. |
| Public items | Every item another context uses becomes `pub`.                                                                                                                                                                                                  | `unreachable_pub` is denied; each crate's surface is the explicit `pub use` list of its `lib.rs` (3.3).                                                                                                                                                                             |
| Test seams   | `#[cfg(test)]` items are invisible to other crates. Two exist today: `Runtime::for_tests` (`providers.rs:271`, used by `turn.rs:1322`, `repl.rs:521`) and the scripted fake backends (D16).                                                     | A crate's fakes of its own ports live in its `testing` module behind a `testing` feature (3.3).                                                                                                                                                                                     |
| Trait impls  | The orphan rule: an `impl` must live with the trait or the type.                                                                                                                                                                                | Wire DTOs are local types with `From` conversions (already the plan). An `impl` that the compiler rejects at extraction moves to the crate that owns the type.                                                                                                                      |
| `unsafe`     | `#![forbid(unsafe_code)]` in every crate root except `shared`.                                                                                                                                                                                  | Check A7.                                                                                                                                                                                                                                                                           |
| Flashboard   | The facade is its own crate: folder `crates/embed`, package `jev-embed`, library `jev_embed` (4.4). These names are final.                                                                                                                      | Exactly one dashboard crate (`jev-usage-io`) depends on it, under the name `jev-engine`.                                                                                                                                                                                            |
| Lints        | One table for all members.                                                                                                                                                                                                                      | `[workspace.lints]` in the root manifest; every member has `[lints] workspace = true` (5.6, check C1).                                                                                                                                                                              |

**Build time.** Measured today on this machine: clean `cargo build` 31 s wall (149 s CPU);
`cargo test --no-run` after it 18 s; rebuild after touching one module 3.5 s (the whole
library recompiles and the eight binaries relink). Expected after the split: the clean build is
dominated by the outside crates and stays within about 20% of today; independent crates of one
level compile in parallel; a change in a leaf crate such as `tui` recompiles that crate and
relinks, a change in `shared` recompiles everything as it does today; `cargo test` links
about 28 small test binaries instead of 9 (16 context libraries, `testkit`, `embed` and its API
test, the eight roots, the architecture test). The numbers are measured again at `ddd-p1` and written here.

**Not chosen.** One crate with a test-enforced map (the earlier recommendation), and three
crates by layer, which would spread every context over three crates.

**Does every crate earn its place?** Checked for the five smallest (production lines today,
tests excluded) and the three that hold no domain. None is merged: each merge would close a
cycle, pull heavy dependencies into a light crate, or create a crate whose purpose needs "and".

| Crate         |      Lines | Why it is not folded into a neighbour                                                                                                                                                                                                       |
| ------------- | ---------: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `chat`        |        311 | It is a second product (`jev-chat`) with its own language (`Transcript`, `Message`). Its only possible hosts are `calibration` (a grab-bag of "small programs") and `tui` (the `jev-code` front end, which would then link a second client) |
| `catalog`     |        391 | Six crates read the model table. It has to sit below `judgment`, so it cannot join `routing`; in `shared` it would make a model-table edit recompile everything                                                                             |
| `routing`     |        473 | Pure rules with no outside dependency. Inside `judgment` they would sit beside an HTTP client, and `Sticky` would return to the place P1a-10 takes it out of                                                                                |
| `calibration` |        517 | Developer tools that depend on `gateway`. Inside `gateway` the proxy would carry the sweeps; inside `judgment` they would close `judgment -> routing -> judgment`                                                                           |
| `doctor`      |        548 | Inside `providers` it would add `providers -> judgment` and a second purpose (reporting) to a state-keeping context                                                                                                                         |
| `embed`       | re-exports | A facade must be a library other workspaces can depend on; `cli` has no library                                                                                                                                                             |
| `testkit`     |    helpers | Shared by every crate's tests; inside `shared` it would ship in the binaries                                                                                                                                                                |
| `cli`         |    8 roots | Keeps the package name `jev-router` and the eight binary names in one manifest                                                                                                                                                              |

The cost that remains is one small `Cargo.toml` and one `README.md` per crate; the template
keeps them identical in shape (4.1, 9.3). A new crate is added only for a new row of 1.2.

### 3.3 Visibility

- A crate's `lib.rs` declares its layers privately (`mod domain; mod application; mod
infrastructure; mod interface;`) and re-exports its surface with an explicit `pub use` list
  (`pub use crate::domain::pin::Pin;`), never a glob. Other crates cannot name a layer module, so rule 2 is a compiler error. This
  holds from step P5-1 on; while the `legacy` module of 8.1 exists it re-exports by glob.
- Items are `pub(crate)` unless `lib.rs` re-exports them. `unreachable_pub` is denied, so a
  stray `pub` is a build failure.
- A crate does not re-export another crate's names (check A10). `embed` is the one exception:
  it is nothing but re-exports.
- **Test doubles.** `jev-testkit` holds helpers that know no context: scratch directory,
  temporary git repository, capture writer, fake Jev HTTP server, the `fixture!` and `golden!`
  macros. It depends on no context crate; if it did, a crate's unit tests would see two
  incompatible copies of that crate's own types. A fake of a context's port (the scripted
  backend, `ProviderRegistry::for_tests`) lives in that crate:
  `#[cfg(any(test, feature = "testing"))] pub mod testing;`. A downstream crate turns the
  feature on in `[dev-dependencies]` only (check C2). A `testing` module is compiled without
  dev-dependencies, so it names only the crate's ordinary dependencies and never `jev_testkit`:
  a fake that needs a scratch directory takes the path as an argument.
- The root manifest sets `resolver = "2"`. A virtual workspace falls back to resolver 1
  (Cargo warns about it), and with resolver 1 a feature turned on by a dev-dependency is also
  turned on in ordinary builds. Tried in a scratch workspace: without the line,
  `cargo build --bin code` compiles the `testing` modules into the binary; with it, it does not.

### 3.4 The architecture test

`tests/architecture.rs` (standard library only, no new dependency; registered as a `[[test]]`
of `crates/cli`, run with `cargo test --test architecture`). With crates, the context map and
the front door are no longer its job. It checks layer direction inside each crate, the
`Cargo.toml` files, and the rules of form.

It is a scanner, not a Rust parser, and it is exact because the source obeys four small rules
that the test itself checks first:

| Source rule                                                                                                                                                                                                                                                           | Why the scanner needs it                     |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| S1. Inside a crate a path to another module starts with `crate::`; a path to another context starts with `jev_<context>::`. `super::` is allowed only to reach the parent of a file in a group folder; `super::super::` and `self::` paths across files are not used. | Every dependency is a literal token sequence |
| S2. One `use` item names one crate: `use jev_routing::{Pin, RouteDecision};`.                                                                                                                                                                                         | The crate is always the first path segment   |
| S3. Outside test code: no glob import of a `crate::` or `jev_*` path, and no `use ... as` rename except `as _` (a trait brought into scope, as `use std::io::Write as _;` today). `embed` may rename, because holding a contract name still is its job.               | A name cannot arrive unseen                  |
| S4. No `macro_rules!` in production code builds a path from fragments.                                                                                                                                                                                                | Paths are visible as written                 |

How it reads a file, in order:

1. Skip test code: the `testkit` crate, `testing` modules, any file under a `tests/` folder,
   `*_tests.rs`, `tests.rs`, `test_facts.rs`, and every item that carries `#[cfg(test)]` or
   `#[cfg(any(test, feature = "testing"))]`, from the attribute to the end of the item (its
   `;` or its matching brace, on the lexed text). Most are `mod tests`; two today are single
   functions (`Runtime::for_tests` in `providers.rs`, `with_liveness` in `delegate.rs`). One production file today has code
   after its test module (`anthropic.rs:159-207`); P0-1 moves it above, and the test then
   requires the test module to be the last item of a file.
2. Lex away what is not code: line comments, nested block comments, string literals, raw
   strings (`r#"..."#` with any number of `#`), byte strings and char literals (told apart
   from lifetimes by the closing quote). The lexer has its own unit tests in the same file,
   including every one of those forms.
3. Expand `use` groups, including nested ones (`use crate::a::{b::{C, D}, e::F};` gives
   `crate::a::b::C`, `crate::a::b::D`, `crate::a::e::F`), then collect every `crate::...` and
   `jev_*::...` path, in `use` items and inline. Where a file imports a module by name
   (`use jev_backend;` or `{self, ...}`), `backend::Name` tokens in that file are collected
   too, since the code calls functions that way today (`log::append`,
   `handoff::SessionInfo`, `providers::backend_for`).
4. Derive the file's crate and layer from its path (`crates/<context>/src/<layer>/...`).
5. For rule 4, build the name-to-layer index from each crate's `lib.rs`: a line
   `pub use crate::domain::pin::Pin;` says `Pin` is a `domain` name (the same `crate::` form
   as everywhere else, rule S1).

The test fails, naming file, line and rule, on:

| Check | Rule                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| A1    | a layer imports a layer it must not (3.1 rule 1)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| A4    | a name of another crate comes from the wrong layer (rule 4)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| A5    | a forbidden outside crate or `std` module for the file's layer (rule 5)                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| A6    | `serde_json::Value`, `serde_json::Map` or `json!` outside the three allow-listed wire files (section 5.2)                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| A7    | the token `unsafe` outside `shared/infrastructure/sys_unix.rs`, test code included (a `forbid` cannot be lifted for a test); a crate root other than `shared`'s without `#![forbid(unsafe_code)]`                                                                                                                                                                                                                                                                                                                                                |
| A8    | any `std::env::` path (or `env::` after `use std::env`) outside `shared/infrastructure/env.rs`; `std::env::args` is also allowed in the binaries. Today: 38 `var`, 6 `var_os`, 8 `args`, 3 `temp_dir`, 2 `current_dir`, 1 `current_exe`, 1 `split_paths`                                                                                                                                                                                                                                                                                         |
| A9    | `Secret::expose` outside `infrastructure/`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| A10   | a `lib.rs` or `mod.rs` that contains anything but docs, attributes, `mod` items and `pub use` lists; a `lib.rs` other than `embed`'s that re-exports a name of another crate                                                                                                                                                                                                                                                                                                                                                                     |
| A11   | a crate folder without `README.md`, or a module without a `//!` doc comment                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| A12   | file names from the banned list (`util`, `utils`, `helpers`, `misc`, `common`, `types`, `manager`, `stuff`)                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| A13   | `dyn Any`, `downcast`, `Box<dyn Error>` or `Result<_, String>` in production code (policies T2 and T4, which no clippy lint covers)                                                                                                                                                                                                                                                                                                                                                                                                              |
| A14   | an `#[allow]` or `#[expect]` that names a lint of 5.6 anywhere but: the fixed test line of 5.6 on a test item; `disallowed_types` in the three wire files; `unsafe_code` on `sys_unix`; `print_stdout`/`print_stderr` in `tui/console/`, an `interface/*_output.rs` file or a root; `exit` in a root. Without this check one attribute would switch a policy off unnoticed                                                                                                                                                                       |
| C1    | for every folder under `crates/`: it is not a workspace member at `crates/<name>/`; its `Cargo.toml` lacks `[lints] workspace = true`; a dependency is not written as `workspace = true`; its internal dependencies differ from its row of 1.3 (the table is a `const` in the test, so widening the map is a reviewed change); `anyhow` outside `crates/cli`, `libc` outside `crates/shared`; two `[[bin]]` targets with one name. The root manifest without `resolver = "2"`, `default-members = ["crates/*"]` or `exclude = [".claude"]` (4.3) |
| C2    | the `testing` feature of a crate enabled under `[dependencies]`; `testkit` depending on a context crate; a `testing` module that names `jev_testkit`; a context crate listed under `[dev-dependencies]` that is not also its dependency                                                                                                                                                                                                                                                                                                          |
| C3    | a dependency a crate lists and never names in its `src/`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| T1    | **only until the crates exist (P0 to P1b):** a flat module imports a module whose context is not in its row of 1.3. The module-to-context table is the "Current path" column of 4.5. Deleted with the last flat module.                                                                                                                                                                                                                                                                                                                          |

A2 and A3 of the earlier draft (entering another context's layer, importing off the map) are
now compiler and Cargo errors and are not in the test.

**Ratchet.** The baseline is one file per context, `tests/architecture_baseline/<context>.txt`,
so parallel lanes never edit the same file. A line is
`check | file | offending path or token | count`: no line numbers, so editing a file does not
disturb it. The rules:

- A violation that is in the baseline passes. One that is not fails.
- A baseline line with no matching violation fails, so a fixed violation must be removed from
  the list in the same commit.
- `ARCH_BASELINE=write cargo test --test architecture` rewrites the files from the tree. It is
  used by a commit that moves code (the file part of the key changes) and never to hide a new
  violation: `scripts/check.sh` compares the per-check totals with the previous commit
  (`git show HEAD~1:tests/architecture_baseline/`) and fails if any total went up. CI
  therefore checks out with history (`fetch-depth: 0`, 3.6); on a commit with no parent
  baseline the comparison is skipped and says so.
- The totals rule starts at tag `ddd-p1`. During P1b whole files land in a layer folder with
  their mixed contents, which is when most A1 and A5 lines first appear.
- The folder is deleted in P5-6, when every file in it is empty.

### 3.5 The one gate

`scripts/check.sh` is the single command every commit runs, locally and in CI:

```
(first step)                                   hermetic environment, see below
cargo clippy --all-targets -- -D warnings      rustc and clippy warnings are errors
cargo test                                     includes the architecture test, embed_api
                                               and the golden tests (4.1)
cargo build --bin code && python3 -I scripts/test_pty_ux.py
python3 -I scripts/test_orch_report.py
python3 -I scripts/test_task_type_accuracy.py
python3 -I scripts/test_provider_eval.py
python3 -I scripts/budget.py                   file and function size (9.2)
(last step)                                    baseline totals did not go up (3.4)
```

No command says `--workspace`. At the root of a virtual workspace Cargo acts on the
`default-members`, which are `crates/*` from P1b-0 on, so the gate covers every engine crate
and nothing else. This matters once Flashboard's three crates are members (4.4): they need
webkit2gtk and a built UI, and `--workspace` ignores `default-members` (tried: with a member
outside `default-members`, `cargo check` skips it and `cargo check --workspace` builds it).

Hermetic environment, set by the script before anything runs, so that a local run and a CI
run see the same machine:

| Setting                                                                                                                                                                   | Why                                                                                                                |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| unset `JEV_API_KEY`, `TYPESAFE_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_GENAI_USE_VERTEXAI`, `GOOGLE_GENAI_USE_GCA`, every `ANTHROPIC_*` | every key and auth variable the code reads (`envfile::lookup` call sites and `gemini::AUTH_ENV_VARS`)              |
| `JEV_ROUTER_ENV` = an empty temporary file                                                                                                                                | otherwise `envfile::lookup` falls back to the developer's `~/.jev-router.env`, which holds real keys               |
| `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`                                                                                                                    | the developer's git identity, signing and hooks settings cannot change a test                                      |
| `CARGO_TARGET_DIR` unset                                                                                                                                                  | `scripts/test_pty_ux.py`, `scripts/install.sh` and `scripts/provider_eval.py` look for binaries in `<root>/target` |

As of `296bd5b`, `cargo test` passed with exactly these settings; recheck at `ddd-start`.

Facts checked as of `296bd5b`; they need checking again on toolchain 1.95.0 and clippy 0.1.95:

- `cargo clippy -- -D warnings` already turns rustc warnings into errors, so there is no
  separate `RUSTFLAGS=-D warnings` build. Setting `RUSTFLAGS` for one command and not the next
  would recompile every dependency twice per run.
- The form `python3 -I -m unittest scripts/test_provider_eval.py` named in that file's docstring
  fails under `-I` (`No module named 'scripts'`); running the file directly passes its 30 tests.
- Cost of one run with a warm build: clippy about 30 s, `cargo test` about 5 s, the PTY suite
  74 s (35 scenarios), the Python tests under 1 s. About two minutes per commit.
- As of `296bd5b`, the tree was not clippy-clean: 23 findings at the default level (11 lints, one of them
  `items_after_test_module` for `anthropic.rs`). P0-1 fixes them, so the gate can deny
  warnings from its first commit.

The P0-0 task reports about 35 warnings across about 30 lints on clippy 0.1.95. That
report has not been verified here.

None of these starts a real provider CLI or spends quota: the Rust tests use fake `sh` scripts
and a fake Jev server, the PTY test uses a fake `claude`.

### 3.6 CI (decision U4)

`.github/workflows/check.yml` in `github.com/putraridho/jev-switch`, added in P0-1:

| Item                      | Value                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Triggers                  | `push` and `pull_request`                                                                                                                                                                                                                                                                                                                                                                |
| Job                       | one job on `ubuntu-latest`: check out with `fetch-depth: 0` (the ratchet reads the previous commit, `move_check.py` reads the diff), let `rustup` install the toolchain named in `rust-toolchain.toml` (`channel = "1.95.0"`, `components = ["clippy"]`; pinned, so a new clippy release cannot fail an old commit, decision U10), restore the cargo cache, run `scripts/check.sh`       |
| Secrets                   | none. The workflow reads no secret and declares `permissions: contents: read`. No `JEV_API_KEY`, no provider key                                                                                                                                                                                                                                                                         |
| Provider CLIs and Jev     | not installed and not reachable. This is the test of hermeticity: a test that would need `claude`, `codex`, `gemini`, `agy`, `grok` or the Jev service fails in CI, and is then faked like the others, never skipped silently. `check.sh` makes a developer's machine look the same: it clears every key variable and points the env file at an empty file before it runs anything (3.5) |
| What the runner must have | `git`, `python3`, `sh`, `date`: all present on `ubuntu-latest`. Tests that commit pass `-c user.name=... -c user.email=...` themselves (`turn.rs:2125`, `delegate.rs:2166`)                                                                                                                                                                                                              |
| Not in CI                 | `scripts/parity.py` (needs the `ddd-start` build; run at phase ends, 8.4) and every eval script that spends quota (`provider_eval.py`, `task_type_accuracy.py`, `orch_report.py` against live logs)                                                                                                                                                                                      |

---

## 4. Target folder tree

### 4.1 Conventions

| Topic                           | Rule                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Context crate                   | `crates/<context>/` with `Cargo.toml`, `README.md` and `src/`. In `src/`: `lib.rs`, `error.rs` when the context has a fallible operation (only `doctor` has none: `routing` has `Pin::parse`, `shared` has `ProviderId::from_str`, `tui` has the three `Result<_, String>` of `repl.rs` and `bin/code.rs`), then those of `domain/`, `application/`, `infrastructure/`, `interface/` that are not empty. Package `jev-<context>`, library `jev_<context>`                                                                                                                                                                                                                                              |
| `Cargo.toml` of a member        | Ten to twenty lines, the same shape everywhere: `[package]` (name, `version.workspace`, `edition.workspace`, `publish = false`), `[dependencies]` with every entry `workspace = true` (versions and features are written once, in the root `[workspace.dependencies]`), optional `[features] testing = []`, `[dev-dependencies]`, `[lints] workspace = true`                                                                                                                                                                                                                                                                                                                                           |
| `lib.rs` and `mod.rs`           | A crate root is `lib.rs`; a folder module uses `mod.rs` (this is what `backend/` and `uifmt/` already do). Both hold only the `//!` doc, attributes, `mod` declarations and the explicit `pub use` list. Logic never lives in them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| File name                       | A singular noun from the glossary for a type (`snapshot.rs`, `route_decision.rs`), a verb phrase for a use case (`undo_turn.rs`, `resume_session.rs`). One concept per file.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Same role, same name            | `ports.rs` (traits a layer needs), `error.rs` (the context's error), `config.rs` (the context's typed settings and how they are read), `<thing>_wire.rs` (serde DTOs for a format), `<thing>_file.rs` (a file adapter), `<thing>_http.rs` (an HTTP adapter), `<thing>_cli.rs` (a CLI adapter), `git_<thing>.rs` (a git adapter), `compose.rs` (wiring)                                                                                                                                                                                                                                                                                                                                                 |
| Banned names                    | `util`, `utils`, `helpers`, `misc`, `common`, `types`, `manager`, `stuff`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Module doc                      | Every file starts with `//!` saying what it **owns** and what it **must not** do (for example: "Owns the cooldown length rules. Must not read the clock or the disk.").                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Errors                          | One `Error` enum per context in `<context>/error.rs`, hand-written `Display` and `std::error::Error` (no new dependency). Variants are closed and typed; no `String`-only "other" variant except where the text is a provider's own message. `anyhow` only in `crates/cli/src/bin/*`.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Unit tests                      | In the file, `#[cfg(test)] mod tests`, when the file stays within the size budget. Otherwise in `<name>_tests.rs` beside it (`#[cfg(test)] #[path = "<name>_tests.rs"] mod tests;`), which keeps access to private items.                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Use-case tests                  | `application/tests/` in the context (`mod.rs` holds the fakes, one file per behaviour).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Test doubles shared by contexts | `crates/testkit/` for helpers that know no context (scratch directory, temporary git repository, fake Jev server, capture writer); a context's own fakes in its `testing` module behind the `testing` feature (3.3). Together they replace the private copies listed in 7.2, D16.                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Fixtures                        | `tests/fixtures/<cli>/` stays at the repository root, where it is today. The 24 fixture `include_str!` paths are relative to the source file and break on every move, so tests load fixtures with `fixture!("claude/turn1.jsonl")` from `jev-testkit`. It expands to `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/", ...))`; `CARGO_MANIFEST_DIR` is evaluated in the calling crate (tried), and every engine crate sits two levels below the root (check C1 requires `crates/<name>/`). A 25th relative `include_str!` in `learn.rs` loads `docs/CHANGELOG.md` and also needs a path that survives the move. |
| Workspace-level tests           | `tests/architecture.rs` at the root, registered in `crates/cli/Cargo.toml` (`[[test]] name = "architecture"`, `path = "../../tests/architecture.rs"`; tried in a scratch workspace, a virtual root cannot own a test). `crates/embed/tests/embed_api.rs` belongs to the facade. An integration test sees only `pub` items, so it cannot reach a wire struct.                                                                                                                                                                                                                                                                                                                                           |
| Golden bytes                    | The expected bytes of every file, log and wire format are data files in `tests/golden/<context>/<format>.txt`, written once in P0-3 and never edited afterwards (`git diff ddd-base -- tests/golden` stays empty). The test that compares is a unit test named `golden_<format>` beside the code that writes the format, so it moves with that code and keeps access to private items; it loads the file with `golden!("turn/turn_log_row.txt")`, built like `fixture!`. Values that differ per run (`ts_ms`, process ids, temporary paths) are replaced by a fixed token in the test before comparing; each golden's first comment line lists what is masked.                                         |

### 4.2 Cross-cutting concerns

| Concern                                | One home                                                                                                                                                                                                                                            | Replaces                                                                                                                        |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Environment variables and the env file | `shared/infrastructure/env.rs`: the only caller of `std::env`. A closed `EnvVar` enum lists every variable the program reads (about 57 names today). Each context has a `config.rs` that turns `&dyn Env` into its typed `Config` once, at startup. | 96 `std::env::` occurrences in production code across 38 files, counting before the first `#[cfg(test)]` per file; this does not count `env::` aliases                                               |
| Secrets                                | `shared/domain/secret.rs`: `Secret` wraps a key; `Debug` and `Display` print `[redacted]`; it has no `Serialize`; `expose()` is callable only from `infrastructure/` (check A9). Every API key travels as `Secret`.                                 | bare `String` keys (`providers.rs:43`, `typesafe.rs:151`)                                                                       |
| State directory                        | `shared/infrastructure/state_dir.rs` behind the `StateDir` port.                                                                                                                                                                                    | `log::state_dir()` called from 8 modules                                                                                        |
| Clock                                  | `shared/application/ports.rs`: `trait Clock { fn now(&self) -> UnixMillis; }`, `shared/infrastructure/system_clock.rs`.                                                                                                                             | `log::now_ms` (`log.rs:28`), `doctor::now_ms` (`doctor.rs:543`), two `Clock` closure aliases (`turn.rs:300`, `delegate.rs:979`) |
| JSON-lines logs                        | `shared/infrastructure/jsonl_log.rs`: append with rotation, generic over `T: Serialize`.                                                                                                                                                            | `log::append(&Value)`                                                                                                           |
| Private atomic file writes             | `shared/infrastructure/private_file.rs`: write to a temporary file with mode 0600, then rename.                                                                                                                                                     | five copies (7.2, row D3)                                                                                                       |
| git                                    | `shared/infrastructure/git_command.rs`: one runner (directory, optional index, optional stdin, typed failure).                                                                                                                                      | four runners (7.2, row D1)                                                                                                      |
| Unsafe system calls                    | `shared/infrastructure/sys_unix.rs` (section 5.5).                                                                                                                                                                                                  | `unsafe` in three files                                                                                                         |
| Poison-tolerant locking                | `shared/domain/lock.rs`: `lock(&Mutex<T>)`.                                                                                                                                                                                                         | nine hand-written copies (7.2, row D7)                                                                                          |

### 4.3 The tree

```
jev-router/
├── Cargo.toml                    virtual manifest: [workspace] members = ["crates/*"],
│                                 default-members = ["crates/*"], exclude = [".claude"],
│                                 resolver = "2"; [workspace.package] (version, edition),
│                                 [workspace.dependencies], [workspace.lints]
├── Cargo.lock                    one lock file for the workspace
├── clippy.toml                   disallowed types and macros, test allowances
├── rust-toolchain.toml           the pinned toolchain (local and CI)                NEW
├── .github/workflows/check.yml   runs scripts/check.sh on push and pull request     NEW
├── README.md
├── docs/architecture/DDD.md      this document
├── eval/  mods/                  unchanged
├── scripts/
│   ├── install.sh                unchanged, byte for byte (see below the tree)
│   ├── check.sh                  the one gate (3.5)                               NEW
│   ├── budget.py                 size budgets (9.2)                               NEW
│   ├── parity.py                 old-binary vs new-binary output comparison (8.4) NEW
│   ├── move_check.py             proves a commit only moved lines (8.1)           NEW
│   └── ...                       python evals and tests, unchanged
├── tests/
│   ├── architecture.rs           layer direction, manifests, rules of form (3.4)  NEW
│   ├── architecture_baseline/    the ratchet, one file per context, deleted last  NEW
│   ├── golden/<context>/         expected bytes of every file, log, wire format   NEW
│   └── fixtures/                 unchanged
└── crates/                       one folder per crate. In the tree below the `src/` level is
    │                             left out: `shared/domain/secret.rs` is
    │                             `crates/shared/src/domain/secret.rs`. `Cargo.toml` and
    │                             `README.md` sit beside `src/`.
    │
    ├── shared/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           provider_id.rs  model_id.rs  probability.rs  unix_millis.rs
    │   │                     token_usage.rs  secret.rs  opaque_json.rs  truncation.rs
    │   │                     duration_text.rs  lock.rs
    │   ├── application/      ports.rs                      (Clock, Env, StateDir)
    │   └── infrastructure/   env.rs  env_file.rs  state_dir.rs  system_clock.rs  jsonl_log.rs
    │                         private_file.rs  git_command.rs  sys_unix.rs
    │
    ├── catalog/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           task_type.rs  tier.rs  legacy_tier.rs  model_spec.rs  model_table.rs
    │   │                     chain.rs  overrides.rs  usable.rs  allowances.rs  pick.rs
    │   └── infrastructure/   chains_file.rs  chains_wire.rs
    │
    ├── judgment/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           question.rs  answer.rs  judgment.rs  signals.rs  thresholds.rs
    │   │                     question_catalog.rs  orchestration_judgment.rs
    │   │                     security_keywords.rs  short_follow_up.rs  tier_want.rs
    │   ├── application/      ports.rs  judge.rs  judge_worker.rs  verify_answer.rs
    │   └── infrastructure/   typesafe_http.rs  typesafe_wire.rs  signals_wire.rs  config.rs
    │
    ├── routing/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   └── domain/           pin.rs  route_input.rs  routing_state.rs  route_decision.rs
    │                         decide.rs  pinned_choice.rs  auto_choice.rs  switch_gate.rs
    │                         plan_steer.rs  legacy_tier_route.rs
    │
    ├── approval/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           permission_request.rs  tool_input.rs  decision.rs  review.rs
    │   │                     visible_text.rs  permission_rule.rs  bash_rule.rs  rule_policy.rs
    │   │                     always_plan.rs  listing.rs
    │   ├── application/      ports.rs  remember_rule.rs  list_rules.rs
    │   └── infrastructure/   claude_settings_file.rs  claude_settings_wire.rs  directory_path.rs
    │
    ├── backend/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           turn_request.rs  turn_outcome.rs  turn_error.rs  error_kind.rs
    │   │                     error_text.rs  limit_text.rs  event.rs  tool_call.rs  plan_usage.rs
    │   │                     mcp_attachment.rs  prompt_arg.rs
    │   ├── application/      ports.rs  ask_once.rs       (TurnRunner, Approver, BackendFactory)
    │   ├── infrastructure/
    │   │   ├── cli_dialect.rs  cli_turn_runner.rs  backend_factory.rs  config.rs
    │   │   ├── path_lookup.rs  mise_install.rs  outcome_wire.rs  lenient.rs
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
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           availability.rs  cooldowns.rs  cooldown_length.rs  reset_time.rs
    │   │                     block_reason.rs  reaction.rs  allowances_rule.rs  setup_guide.rs
    │   │                     late_probe.rs
    │   │                     probe_facts/  claude.rs  codex.rs  gemini.rs  grok.rs  deepseek.rs
    │   ├── application/      ports.rs  provider_registry.rs  probe_providers.rs  retry_slow_probes.rs
    │   └── infrastructure/   cooldown_file.rs  cooldown_wire.rs  system_date.rs  config.rs
    │                         probes/  claude_probe.rs  codex_probe.rs  codex_auth_wire.rs
    │                                  gemini_probe.rs  grok_probe.rs  deepseek_probe.rs
    │
    ├── session/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           session.rs  turn_record.rs  catch_up.rs  catch_up_level.rs
    │   │                     tool_trail.rs  saved_session.rs  session_info.rs  project_key.rs
    │   ├── application/      ports.rs  save_session.rs  resume_session.rs
    │   └── infrastructure/   session_file.rs  session_wire.rs
    │
    ├── changes/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           snapshot.rs  file_change.rs  change_set.rs  untracked_merge.rs
    │   │                     undo_plan.rs  undo_report.rs  new_file_diff.rs  safe_path.rs
    │   ├── application/      ports.rs  track_turn.rs  undo_turn.rs  show_diff.rs
    │   └── infrastructure/   git_working_tree.rs  git_temp_index.rs  git_output.rs  undo_files.rs
    │
    ├── orchestration/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           orchestration_gate.rs  planner_choice.rs  planner_note.rs
    │   │                     brief_signals.rs  worker_pick.rs  worker.rs  worker_prompt.rs
    │   │                     delegation.rs  delegation_result.rs  file_patch.rs  patch_page.rs
    │   │                     recovery.rs
    │   ├── application/      ports.rs  delegator.rs  delegate_work.rs  run_worker.rs
    │   │                     show_worker_diff.rs  merge_worker.rs  discard_worker.rs
    │   │                     recover_workers.rs  reservation.rs  attach_delegate_tool.rs
    │   ├── infrastructure/   git_worktrees.rs  worktree_base.rs  worker_file.rs  worker_wire.rs
    │   │                     process_liveness.rs  delegate_log.rs  delegate_log_wire.rs  config.rs
    │   └── interface/mcp/    mod.rs  json_rpc.rs  tool_schema.rs  tool_arguments.rs
    │                         mcp_server.rs  lazy_delegator.rs  compose.rs
    │
    ├── turn/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           turn_report.rs  judged.rs  attempt.rs  failover.rs  no_route_info.rs
    │   ├── application/      ports.rs  engine.rs  engine_builder.rs  cancel.rs  run_turn.rs
    │   │                     run_attempts.rs  choose_model.rs  plan_orchestration.rs
    │   │                     run_attempt.rs  finish_turn.rs  approve_tool.rs
    │   │                     session_commands.rs  change_commands.rs
    │   └── infrastructure/   turn_log.rs  turn_log_wire.rs  git_status.rs  config.rs
    │
    ├── doctor/
    │   ├── Cargo.toml  README.md  lib.rs
    │   ├── domain/           check.rs  report.rs  inputs.rs  diagnose.rs  provider_checks.rs
    │   │                     machine_checks.rs  first_run_hint.rs
    │   ├── application/      ports.rs  gather_inputs.rs
    │   └── infrastructure/   machine_facts.rs
    │
    ├── gateway/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
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
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           message.rs  transcript.rs
    │   ├── application/      ports.rs  chat_turn.rs
    │   ├── infrastructure/   claude_print_cli.rs  claude_print_wire.rs
    │   └── interface/        chat_repl.rs
    │
    ├── calibration/
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
    │   ├── domain/           threshold_sweep.rs  prompt_item.rs  score_band.rs
    │   ├── application/      sweep_cheap_threshold.rs  dry_run_routes.rs  sweep_orchestration_cutoff.rs
    │   └── infrastructure/   cases_file.rs  prompt_file.rs  gateway_log_reader.rs
    │
    ├── tui/                  interface only: the terminal front end of jev-code
    │   ├── Cargo.toml  README.md  lib.rs  error.rs
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
    ├── embed/                the stable facade for Flashboard (4.4)
    │   ├── Cargo.toml  README.md  lib.rs      re-exports only
    │   └── tests/embed_api.rs                 pins the name and signature of every export
    │
    ├── testkit/              test helpers that know no context (3.3); dev-dependency only
    │   ├── Cargo.toml  README.md  lib.rs
    │   └── scratch_dir.rs  temp_git_repo.rs  capture_writer.rs  fake_jev_server.rs
    │       fixture.rs  golden.rs
    │
    └── cli/                  package `jev-router`: the eight binaries and nothing else
        ├── Cargo.toml        the eight [[bin]] entries, [[test]] architecture
        └── bin/              composition roots only, 30 lines or fewer each
            ├── code.rs           -> jev_tui::run()                      installed as jev-code
            ├── gateway.rs        -> jev_gateway::serve()                installed as jev-gateway
            ├── delegate_mcp.rs   -> jev_orchestration::serve_mcp()      installed as jev-delegate-mcp
            ├── ask.rs            -> jev_backend::ask_once()             installed as jev-ask
            ├── chat.rs           -> jev_chat::run()                     installed as jev-chat
            └── tune.rs  route.rs  orch_tune.rs   -> jev_calibration
```

**Binaries and `scripts/install.sh`.** The script runs
`cargo build --release --bin code --bin ask --bin chat --bin gateway --bin delegate_mcp` at the
repository root and installs `target/release/{code,ask,delegate_mcp,chat,gateway}` as
`jev-code`, `jev-ask`, `jev-delegate-mcp`, `jev-chat`, `jev-gateway`, then copies
`scripts/jev-claude` and `mods/jev-orchestrate`. None of that changes:

| What the script relies on                                           | Why it still holds                                                                                                                                                                                                                                              |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--bin <name>` at the root                                          | The root is a virtual workspace whose `default-members` are `crates/*`, so the command looks in every engine crate and finds the five targets in `crates/cli` (tried). Bin names are unique among them (check C1); a dashboard binary is never a default member |
| `target/release/<name>`                                             | A workspace has one `target/` at its root                                                                                                                                                                                                                       |
| `mods/`, `scripts/jev-claude`                                       | Not moved                                                                                                                                                                                                                                                       |
| `delegate_mcp` beside the running executable (`orchestrate.rs:151`) | Same file names, same folder                                                                                                                                                                                                                                    |

`scripts/install.sh` is therefore not edited (`git diff ddd-base -- scripts/install.sh` is
empty), and the same holds for `scripts/test_pty_ux.py` (`target/debug/code`) and
`scripts/provider_eval.py` (`target/release/ask`). `cargo run --bin code` keeps working.

**The root manifest's three workspace lines**, each tried in a scratch workspace:

| Line                             | Without it                                                                                                                                                                                                                                                                                                    |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `resolver = "2"`                 | Cargo warns and uses resolver 1, which compiles every `testing` module into the release binaries (3.3)                                                                                                                                                                                                        |
| `default-members = ["crates/*"]` | Root commands would also build any later member, such as Flashboard's crates (3.5, 4.4)                                                                                                                                                                                                                       |
| `exclude = [".claude"]`          | Agent worktrees of other branches live in `.claude/worktrees/` inside this repository. A worktree whose own manifest has no `[workspace]` table then fails every cargo command with "current package believes it's in a workspace when it's not". The dashboard design adds the same line for the same reason |

**Version.** `jev-code --version`, `jev-chat --version`, `jev-gateway --version` and the MCP
server's `serverInfo` print `env!("CARGO_PKG_VERSION")` (`bin/code.rs:107`, `bin/chat.rs:45`,
`bin/gateway.rs:114`, `bin/delegate_mcp.rs:164`). After the split that expression is compiled
in `tui`, `chat`, `gateway` and `orchestration`. Every member takes `version.workspace = true`,
so the printed text stays `0.1.0` and a release is one edit of `[workspace.package]`; the
parity run (8.4) compares `--version`.

### 4.4 Public API and Flashboard

`crates/embed` (package `jev-embed`, library `jev_embed`) is the engine's one stable surface
for other programs. These three names are final. It contains re-exports only, and
`crates/embed/tests/embed_api.rs` pins every export by name and by signature
(`let _: fn() -> Option<PathBuf> = jev_embed::state_dir;`), so removing, renaming or
re-typing one is a failing engine test, not a surprise in the app.

**The contract with Flashboard** (dashboard design, branch `dashboard`,
`docs/dashboard/DESIGN.md` sections 5 and 5.1). Flashboard owns a port, the module
`jev-usage-io::engine_bridge`, and that file alone names the engine. Behind it the facade must
offer six names with these signatures, all of which exist today:

| Facade name         | Signature                                               | Today                                             | After the refactor                      |
| ------------------- | ------------------------------------------------------- | ------------------------------------------------- | --------------------------------------- |
| `ProviderId`        | enum                                                    | `backend::ProviderId`                             | `shared/domain/provider_id.rs`          |
| `state_dir`         | `fn() -> Option<PathBuf>`                               | `log::state_dir`                                  | `shared/infrastructure/state_dir.rs`    |
| `env_file_path`     | `fn(Option<String>, Option<String>) -> Option<PathBuf>` | `envfile::env_file_path`                          | `shared/infrastructure/env_file.rs`     |
| `parse_env`         | `fn(&str, &str) -> Option<String>`                      | `envfile::parse`                                  | `shared/infrastructure/env_file.rs`     |
| `parse_codex_login` | `fn(bool, &str) -> Option<bool>`                        | `availability::parse_codex_login` (private today) | `providers/domain/probe_facts/codex.rs` |
| `login_command`     | `fn(ProviderId) -> Option<&'static str>`                | `doctor::login_command`                           | `providers/domain/setup_guide.rs`       |

Consequences inside the engine:

- These six stay free functions with these signatures through every phase. The `StateDir` and
  `Env` ports of P3 are added around them; P3-4 deletes the callers' use of `log::state_dir`
  and `envfile::lookup`, not the three functions the facade exports. `embed_api.rs` exists
  from P1b-19 on, so a step that breaks one fails the gate.
- The facade exports nothing else to the dashboard. The dashboard design deliberately does not
  call `probe_all`, `path_from_env` or the registry (they read more of the machine than a
  resident app may), so the earlier list (`Runtime`, `ALL_PROVIDERS`, `probe_all`,
  `Availability`, `install_hint`, ...) is dropped and no `Runtime` alias is kept.
- For the later chat window (dashboard design section 15, its own port in a future
  `jev-chat-io`) P5-4 adds `Engine`, `EngineBuilder`, `TurnObserver`, `TurnReport`,
  `TurnStatus`, `Cancel`, `Event`, `PermissionRequest`, `Decision`, `Pin`, `ChangeSet`,
  `SessionInfo`, each pinned in `embed_api.rs` the same way.
- Binaries import crate roots (`jev_turn::Engine`), never layer modules (they cannot).

**The two lines.** The dashboard writes the engine's crate name and path in exactly two
places, and renames the dependency so its code says `jev_engine` whatever the facade is called:

```toml
# dashboard/crates/usage-io/Cargo.toml
jev-engine = { package = "jev-embed", path = "../../../crates/embed" }
```

```rust
// dashboard/crates/usage-io/src/engine_bridge.rs
use jev_engine as engine;
```

**Both orders are valid.** The dashboard design allows its first task (T00) to run before or
after this refactor.

|                          | T00 ran before the refactor (design "Path A")                                                                                                                                                                                                                                         | T00 runs after it ("Path B")                                                                                                                             |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What exists at P0        | `src/embed.rs` (six `pub use` lines), `pub mod embed;`, `pub` on `parse_codex_login`, a root `[workspace]` with the three app members, `default-members = ["."]`, `exclude`, `[profile.dist]`; the two lines name `package = "jev-router", path = "../../.."` and `jev_engine::embed` | nothing of Flashboard                                                                                                                                    |
| P1b-0                    | the root keeps its `[workspace]`, `exclude` and `[profile.dist]` tables; `members` becomes `crates/*` plus the three app crates, `default-members` becomes `crates/*`; the dashboard's path becomes `../../../crates/cli` (the package moved)                                         | the root manifest is created as in 4.3                                                                                                                   |
| P1b-19                   | `git mv crates/cli/src/embed.rs crates/embed/src/lib.rs`; the two lines take the final form above                                                                                                                                                                                     | `crates/embed/src/lib.rs` is written new                                                                                                                 |
| Check in those two steps | `cargo test -p jev-usage-io --test engine_bridge` (the dashboard's contract tests; `usage-io` needs no Tauri library)                                                                                                                                                                 | none                                                                                                                                                     |
| Afterwards               | nothing                                                                                                                                                                                                                                                                               | T00 appends its three members, its `exclude` entry for `dashboard/fuzz` and `[profile.dist]`, writes the two lines, and does not touch `default-members` |

Those are the only edits this refactor makes under `dashboard/`, and only in Path A.

**What each side owns in the shared workspace** (one `Cargo.lock`, one `target/`):

| Item                                                                      | Owner     | Rule                                                                                                                                                                                                              |
| ------------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `default-members`                                                         | engine    | Always `crates/*`: root `cargo build`, `cargo test`, `scripts/install.sh` and `scripts/check.sh` never build an app crate (the dashboard design's invariants 1 and 5). No engine command says `--workspace` (3.5) |
| `[workspace.lints]`, root `clippy.toml`                                   | engine    | The app crates do not opt in (the dashboard design's invariant 3): each carries its own `[lints]` table, and `dashboard/clippy.toml` is the nearest one for them. Check C1 looks only at folders under `crates/`  |
| `[profile.dist]`, the app members, `exclude` entries for `dashboard/fuzz` | dashboard | The engine sets no profile                                                                                                                                                                                        |
| engine crates                                                             | engine    | A dashboard `Cargo.toml` names no `jev-*` engine crate except `jev-embed` in `usage-io`; if the facade lacks a name, that is a request to the engine, never an edit of an engine crate from the app               |

### 4.5 Every current file and where it goes

This table is as of `296bd5b`. It does not yet list every file at `ddd-start`.

"Split" means the file becomes several files in the listed folder; the first file named keeps
the `git mv` history (it holds the largest part). New paths are under `crates/`, with `src/`
left out as in the tree.

| Current path                                                                                                                | Lines | New path                                      | Split or merge, and why                                                                                                                                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------- | ----: | --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`                                                                                                                |    41 | deleted                                       | Each crate has its own `lib.rs`; the crate doc moves to the root `README.md`                                                                                                                                                                                                                                                                                                                                                  |
| `Cargo.toml`                                                                                                                |    51 | `Cargo.toml` (virtual)                        | The `[[bin]]` entries move to `crates/cli/Cargo.toml`; dependencies to `[workspace.dependencies]`                                                                                                                                                                                                                                                                                                                             |
| `src/envfile.rs`                                                                                                            |   121 | `shared/infrastructure/env_file.rs`           | Pure parsing stays with it; `lookup` becomes the `Env` port in `env.rs`                                                                                                                                                                                                                                                                                                                                                       |
| `src/log.rs`                                                                                                                |    59 | `shared/infrastructure/jsonl_log.rs`          | Split: `state_dir` to `state_dir.rs`, `now_ms` to `system_clock.rs`. It mixed three concerns                                                                                                                                                                                                                                                                                                                                  |
| `src/base64.rs`                                                                                                            |    32 | `shared/domain/base64.rs`               | Move shared encoding used by backend and terminal. |
| `src/ext/sha256.rs`                                                                                                        |    88 | `shared/domain/sha256.rs`               | Move context-free checksum calculation. |
| `src/redact.rs`                                                                                                            |   100 | `shared/domain/redact.rs`               | Move context-free redaction rules. |
| `src/secrets.rs`                                                                                                           |   494 | `shared/domain/secrets.rs`              | Move pure secret detection rules shared by privacy and redaction. |
| `src/typesafe.rs`                                                                                                           |   188 | `judgment/infrastructure/typesafe_http.rs`    | Split: request and response shapes to `typesafe_wire.rs`; `ChoiceAnswer` to `judgment/domain/answer.rs`                                                                                                                                                                                                                                                                                                                       |
| `src/router.rs`                                                                                                             |  1197 | `judgment/application/judge.rs`               | Split, see 7.3; `Sticky` is the one part that leaves the context (to `routing`)                                                                                                                                                                                                                                                                                                                                               |
| `src/catalog.rs`                                                                                                            |   576 | `catalog/domain/model_table.rs`               | Split by concept (`task_type`, `tier`, `model_spec`, `chain`, `overrides`, `usable`, `allowances`, `pick`); `Overrides::from_env` (reads a file, `catalog.rs:252`) to `infrastructure/chains_file.rs`                                                                                                                                                                                                                         |
| `src/routing.rs`                                                                                                            |   872 | `routing/domain/decide.rs`                    | Split: `pin`, `route_input`, `routing_state`, `route_decision`, `pinned_choice`, `auto_choice`, `switch_gate`, `plan_steer`                                                                                                                                                                                                                                                                                                   |
| `src/routing_rules.rs`                                                                                                     |   310 | `routing/domain/user_rules.rs`          | Split routing policy from file IO in turn infrastructure; move shared path matching out of privacy to avoid a cycle. |
| `src/anthropic.rs`                                                                                                          |   226 | `chat/infrastructure/claude_print_cli.rs`     | Split three ways: model id constants to `catalog/domain/legacy_tier.rs`; `is_limit_phrase`/`is_limit_message` to `backend/domain/limit_text.rs`; `Message` to `chat/domain/message.rs`. A chat client, a model list and an error classifier shared one file                                                                                                                                                                   |
| `src/transcript.rs`                                                                                                         |    33 | `chat/domain/transcript.rs`                   | Move                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/backend/mod.rs`                                                                                                        |  1874 | `backend/infrastructure/process/child.rs`     | Split, see 7.3                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/backend/claude.rs`                                                                                                     |   564 | `backend/infrastructure/claude/claude_cli.rs` | Split: typed events to `claude_wire.rs`, approval lines to `permission_wire.rs`, MCP config to `mcp_config.rs`                                                                                                                                                                                                                                                                                                                |
| `src/backend/codex.rs`                                                                                                      |   482 | `backend/infrastructure/codex/codex_cli.rs`   | Split: `codex_wire.rs`                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src/backend/gemini.rs`                                                                                                     |   718 | `backend/infrastructure/gemini/gemini_cli.rs` | Split: `gemini_wire.rs`, `agy_wire.rs`, `flavor.rs` (two CLIs, one provider)                                                                                                                                                                                                                                                                                                                                                  |
| `src/backend/grok.rs`                                                                                                       |   408 | `backend/infrastructure/grok/grok_cli.rs`     | Split: `grok_wire.rs`                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/providers.rs`                                                                                                          |   904 | `providers/application/provider_registry.rs`  | Split: backend construction (`backend_for`, `deepseek_env`, gemini flavor, lines 43-116) to `backend/infrastructure/backend_factory.rs`; `react` rules to `domain/reaction.rs`; late probes to `retry_slow_probes.rs`; `render*` and `format_left` to `tui/render/providers_text.rs` and `shared/domain/duration_text.rs`                                                                                                     |
| `src/availability.rs`                                                                                                       |   895 | `providers/domain/availability.rs`            | Split: each provider's pure `decide_*`/`parse_*` to `domain/probe_facts/<provider>.rs`, its IO half to `infrastructure/probes/<provider>_probe.rs`; PATH and mise checks to `backend/infrastructure/path_lookup.rs`, `mise_install.rs` (1.3); `render*` to `tui/render`                                                                                                                                                       |
| `src/cooldown.rs`                                                                                                           |   852 | `providers/domain/reset_time.rs`              | Split: `cooldown_length.rs`, `cooldowns.rs` (the set), `infrastructure/cooldown_file.rs` (save, load, `DiskWatch`), `infrastructure/system_date.rs` (the `date` command and zoneinfo)                                                                                                                                                                                                                                         |
| `src/balance.rs`                                                                                                           |   151 | `providers/infrastructure/deepseek_balance.rs`| Split balance facts from the DeepSeek HTTP adapter. |
| `src/planstore.rs`                                                                                                         |    84 | `providers/infrastructure/plan_store.rs`| Split stored plan wire format from file IO and provider facts. |
| `src/probecache.rs`                                                                                                        |   177 | `providers/infrastructure/probe_cache.rs`| Split cache validity rules and availability facts from file IO. |
| `src/handoff.rs`                                                                                                            |  1105 | `session/domain/catch_up.rs`                  | Split: `session.rs` (`Sessions`), `turn_record.rs`, `tool_trail.rs`, `saved_session.rs`, `session_info.rs`, `project_key.rs`; file reading and writing to `infrastructure/session_file.rs`; `write_atomic` to `shared/infrastructure/private_file.rs`                                                                                                                                                                         |
| `src/ctxintel.rs`                                                                                                          |   163 | `session/domain/context_intelligence.rs`| Split context-size and compaction rules from terminal formatting; pass turn facts in as values. |
| `src/export.rs`                                                                                                            |   148 | `session/application/export.rs`         | Split redacted export rules from file writing and HTML rendering adapters. |
| `src/projmemory.rs`                                                                                                        |   149 | `session/infrastructure/project_memory.rs`| No context fits project memory; session is closest for remembered context. Split note rules from state-file IO. |
| `src/replay.rs`                                                                                                            |    86 | `session/application/replay.rs`         | Split replay facts and redaction from tui presentation. |
| `src/repomap.rs`                                                                                                           |   280 | `session/infrastructure/repository_map.rs`| No context fits repository mapping; session is closest for prompt context. Split symbol and budget rules from git and filesystem IO. |
| `src/sessionimport.rs`                                                                                                     |   456 | `session/infrastructure/session_import.rs`| Split import orchestration and session values from external CLI history readers. |
| `src/sessionlib.rs`                                                                                                        |   668 | `session/application/session_library.rs`| Split library queries and marks to domain, persistence to infrastructure, and formatting to tui. |
| `src/sessionmeta.rs`                                                                                                       |   139 | `session/infrastructure/session_meta.rs`| Split metadata and tag rules to domain from file persistence. |
| `src/changes.rs`                                                                                                            |  1586 | `changes/infrastructure/git_working_tree.rs`  | Split, see 7.3                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/diffcmd.rs`                                                                                                           |   265 | `changes/application/diff_command.rs`   | Split diff and undo rules to domain, git and file IO to infrastructure, and display options to tui. |
| `src/diffparse.rs`                                                                                                         |   335 | `changes/domain/diff_parse.rs`          | Move unified diff values and parsing. |
| `src/permissions.rs`                                                                                                        |  1348 | `approval/domain/rule_policy.rs`              | Split, see 7.3                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/ext/frontmatter.rs`                                                                                                   |    94 | `approval/domain/extension_frontmatter.rs`| No context fits extension management; approval is closest for extension trust. Keep parsing separate from trust policy. |
| `src/ext/hooks.rs`                                                                                                         |   781 | `approval/application/extension_hooks.rs`| No context fits extension management; approval is closest for hook trust. Split trust and refusal rules to domain, hook file and process IO to infrastructure. |
| `src/ext/library.rs`                                                                                                       |   423 | `approval/application/extension_library.rs`| No context fits extension management; approval is closest for extension trust. Split library and recipe rules from file IO and turn execution. |
| `src/ext/market.rs`                                                                                                        |   684 | `approval/application/extension_market.rs`| No context fits extension management; approval is closest for extension trust. Split manifest and verification rules from file IO. |
| `src/ext/mod.rs`                                                                                                           |   146 | `approval/application/extensions.rs`    | No context fits extension management; approval is closest for extension trust. Split extension paths and command values from filesystem access. |
| `src/mode.rs`                                                                                                              |   228 | `approval/domain/mode.rs`               | Move permission mode and policy. |
| `src/profile.rs`                                                                                                           |   146 | `approval/domain/profile.rs`            | Move permission profile rules. |
| `src/orchestrate.rs`                                                                                                        |   303 | `orchestration/domain/orchestration_gate.rs`  | Split: `planner_choice.rs`, `planner_note.rs`; `Setup` to `application/attach_delegate_tool.rs`; `write_mcp_config` to `backend/infrastructure/claude/mcp_config.rs` (1.3)                                                                                                                                                                                                                                                    |
| `src/delegate.rs`                                                                                                           |  2856 | `orchestration/application/delegator.rs`      | Split, see 7.3                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/turn.rs`                                                                                                               |  2835 | `turn/application/engine.rs`                  | Split, see 7.3 and 9.4                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src/attach.rs`                                                                                                            |   289 | `turn/application/attach.rs`            | Split path recognition into domain rules and filesystem checks into infrastructure. |
| `src/audit.rs`                                                                                                             |   282 | `turn/infrastructure/audit.rs`          | Split audit record types and redaction rules from JSONL storage and usage aggregation. |
| `src/background.rs`                                                                                                        |   116 | `turn/domain/background.rs`             | Move background-work notice rules. |
| `src/bridge.rs`                                                                                                            |   135 | `turn/interface/editor_bridge.rs`       | No context fits editor integration; turn is closest. Split protocol values from wire parsing. |
| `src/budget.rs`                                                                                                            |   445 | `turn/domain/budget.rs`                 | Split budget and price rules from price-file IO; move display wording to tui. |
| `src/cache.rs`                                                                                                             |   418 | `turn/domain/cache.rs`                  | Split cache rules from terminal formatting in tui; avoid the downstream session-log dependency. |
| `src/effort.rs`                                                                                                            |   183 | `turn/domain/effort.rs`                 | Split effort choice from provider CLI encoding in backend infrastructure and wording in tui. |
| `src/failover.rs`                                                                                                          |   151 | `turn/domain/failover_policy.rs`        | Split failover policy from policy-file IO in turn infrastructure and cost wording in tui. |
| `src/guard.rs`                                                                                                             |   313 | `turn/application/guards.rs`            | Split policy aggregation from audit IO and move formatting to tui; consume routing and approval rules through their public surfaces. |
| `src/inflight.rs`                                                                                                          |   216 | `turn/infrastructure/inflight.rs`       | Split recovery marker values from state-file IO. |
| `src/privacy.rs`                                                                                                           |   844 | `turn/domain/privacy.rs`                | No context fits data-routing privacy; turn is closest for prompt dispatch. Split policy rules from policy-file and audit IO. |
| `src/schedule.rs`                                                                                                          |   307 | `turn/domain/schedule.rs`               | No context fits repeated runs; turn is closest for execution. Split schedule rules from clock and sleeping in turn infrastructure. |
| `src/scoreboard.rs`                                                                                                        |   140 | `turn/domain/scoreboard.rs`             | Move provider outcome aggregation; keep display in tui. |
| `src/sessionlog.rs`                                                                                                        |   195 | `turn/domain/session_log.rs`            | Split session turn facts from export formatting in tui; avoid a session-to-turn dependency. |
| `src/team.rs`                                                                                                              |   200 | `turn/application/team_rules.rs`        | No context fits team configuration; turn is closest for applying cross-context rules. Split routing and approval policy from file IO. |
| `src/turnlog.rs`                                                                                                            |   152 | `backend/infrastructure/outcome_wire.rs`      | Split: `ask_json` to `backend/interface/ask_output.rs`. `turn/infrastructure/turn_log.rs` and `turn_log_wire.rs` are new files made from `Engine::log_attempt` (`turn.rs:1081`)                                                                                                                                                                                                                                               |
| `src/backend/codex_plan.rs`                                                                                                |   179 | `backend/infrastructure/codex/codex_plan.rs`| Split plan-reading wire and file IO from plan facts. |
| `src/backend/gemini/acp.rs`                                                                                                |   587 | `backend/infrastructure/gemini/acp.rs`  | Split ACP wire types from transport and approval dispatch. |
| `src/backend/testing.rs`                                                                                                   |   151 | `backend/testing.rs`                    | Backend port fake; keep behind the testing feature as in 3.3. |
| `src/mcp.rs`                                                                                                               |   312 | `backend/infrastructure/mcp_discovery.rs`| Split MCP server values from provider configuration readers and live status probes. |
| `src/doctor.rs`                                                                                                             |   952 | `doctor/domain/diagnose.rs`                   | Split: `check.rs`, `report.rs`, `inputs.rs`, `provider_checks.rs`, `machine_checks.rs`, `first_run_hint.rs`; `gather` to `application/gather_inputs.rs`; machine probing to `infrastructure/machine_facts.rs`; `install_hint`, `login_command`, `login_instruction` to `providers/domain/setup_guide.rs` together with `providers::login_target` (they are provider knowledge, and this removes the `doctor <-> uifmt` cycle) |
| `src/usage.rs`                                                                                                              |   676 | `gateway/application/usage_tap.rs`            | Split: `turn_rollup.rs` (`Turns`), domain `usage_record.rs`, `gateway_turn.rs`, `similarity.rs`; record shapes to `infrastructure/usage_log_wire.rs`                                                                                                                                                                                                                                                                          |
| `src/prefix.rs`                                                                                                             |   243 | `gateway/domain/prefix_fingerprint.rs`        | Split: `cache_break.rs` (the comparison), `application/prefix_watch.rs` (the recent-request memory)                                                                                                                                                                                                                                                                                                                           |
| `src/bin/gateway.rs`                                                                                                        |  1063 | `gateway/interface/http/proxy_handler.rs`     | Split, see 7.3; `cli/bin/gateway.rs` remains as a 30-line root                                                                                                                                                                                                                                                                                                                                                                |
| `src/bin/delegate_mcp.rs`                                                                                                   |   596 | `orchestration/interface/mcp/mcp_server.rs`   | Split: `json_rpc.rs`, `tool_schema.rs`, `tool_arguments.rs`, `lazy_delegator.rs`, `compose.rs`; root remains                                                                                                                                                                                                                                                                                                                  |
| `src/projectkind.rs`                                                                                                       |   218 | `orchestration/infrastructure/project_kind.rs`| No context fits project detection; orchestration is closest for worker checks. Split check-selection rules from marker-file IO. |
| `src/workercheck.rs`                                                                                                       |   228 | `orchestration/infrastructure/worker_check.rs`| Split check orchestration and timeout policy from process execution. |
| `src/bin/code.rs`                                                                                                           |   405 | `tui/app/interactive_session.rs`              | Split: `compose.rs`, `print_mode.rs`, `startup.rs`, `doctor_command.rs`; root remains                                                                                                                                                                                                                                                                                                                                         |
| `src/bin/ask.rs`                                                                                                            |    52 | `cli/bin/ask.rs`                              | Logic to `backend/application/ask_once.rs`                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/bin/chat.rs`                                                                                                           |   121 | `cli/bin/chat.rs`                             | Loop to `chat/interface/chat_repl.rs`                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/bin/tune.rs`                                                                                                           |   106 | `cli/bin/tune.rs`                             | Logic to `calibration/application/sweep_cheap_threshold.rs`                                                                                                                                                                                                                                                                                                                                                                   |
| `src/bin/route.rs`                                                                                                          |   306 | `cli/bin/route.rs`                            | Logic to `calibration/application/dry_run_routes.rs`                                                                                                                                                                                                                                                                                                                                                                          |
| `src/bin/orch_tune.rs`                                                                                                      |   115 | `cli/bin/orch_tune.rs`                        | Logic to `calibration/application/sweep_orchestration_cutoff.rs`                                                                                                                                                                                                                                                                                                                                                              |
| `src/bench.rs`                                                                                                             |   162 | `calibration/application/bench.rs`      | Split ranking rules to domain, task-file reads to infrastructure and presentation to tui. |
| `src/cli.rs`                                                                                                                |   263 | `tui/cli_args.rs`                             | Move                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/commands.rs`                                                                                                           |   294 | `tui/command.rs`                              | Move                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/console.rs`                                                                                                            |  1433 | `tui/console/console.rs`                      | Split, see 7.3                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/lineedit.rs`                                                                                                           |   575 | `tui/input/line_editor.rs`                    | Split: `completion_helper.rs`, `history_file.rs`, `interrupt_guard.rs`, `answer_key.rs`; raw-mode calls to `shared/infrastructure/sys_unix.rs`                                                                                                                                                                                                                                                                                |
| `src/repl.rs`                                                                                                               |   785 | `tui/repl/repl.rs`                            | Split: `turn_race.rs` (the Ctrl-C race, `SigInt`), `command_handlers.rs`, `pager.rs`                                                                                                                                                                                                                                                                                                                                          |
| `src/views.rs`                                                                                                              |   555 | `tui/views/status_facts.rs`                   | Split one screen per file                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/terminal.rs`                                                                                                           |   215 | `tui/tty/terminal_guard.rs`                   | Split: `terminal_size.rs`, `screen.rs`, `panic_hook.rs`; termios and ioctl calls to `sys_unix.rs`                                                                                                                                                                                                                                                                                                                             |
| `src/uifmt/mod.rs`                                                                                                          |   115 | `tui/render/mod.rs`                           | Move; its two cross-formatter tests go to `render/tests.rs`                                                                                                                                                                                                                                                                                                                                                                   |
| `src/uifmt/commands.rs`                                                                                                     |    80 | `tui/command_table.rs`                        | Move (it is the command table, not a formatter)                                                                                                                                                                                                                                                                                                                                                                               |
| `src/uifmt/complete.rs`                                                                                                     |   227 | `tui/render/completion.rs`                    | Move                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/uifmt/testutil.rs`                                                                                                     |    70 | `tui/render/test_facts.rs`                    | Move                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/agentview/mod.rs`                                                                                                     |    74 | `tui/views/agent_render.rs`             | Move orchestration presentation. |
| `src/agentview/orch.rs`                                                                                                    |   464 | `tui/views/agent_orch.rs`               | Move orchestration presentation. |
| `src/agentview/stored.rs`                                                                                                  |    71 | `tui/views/agent_stored.rs`             | Split worker-record loading into orchestration infrastructure; keep view facts and rendering in tui. |
| `src/agentview/tasks.rs`                                                                                                   |    83 | `tui/views/agent_tasks.rs`              | Move orchestration presentation. |
| `src/agentview/workers.rs`                                                                                                 |   468 | `tui/views/agent_workers.rs`            | Move orchestration presentation. |
| `src/bgquery.rs`                                                                                                           |    73 | `tui/tty/background_query.rs`           | Move terminal background probe. |
| `src/ext/cli.rs`                                                                                                           |   155 | `tui/app/extension_cli.rs`              | No context fits extension management; tui is closest for CLI delivery. Split command parsing from installation IO in approval. |
| `src/ext/view.rs`                                                                                                          |   240 | `tui/render/extension_view.rs`          | Move extension command wording and presentation. |
| `src/extcmds.rs`                                                                                                           |   122 | `tui/repl/extended_commands.rs`         | Move command parsing and help wiring. |
| `src/front.rs`                                                                                                             |   167 | `tui/console/front.rs`                  | Move typed terminal front channel. |
| `src/gimmicks/autopsy.rs`                                                                                                  |   100 | `tui/render/autopsy.rs`                 | Move terminal feature. |
| `src/gimmicks/gauge.rs`                                                                                                    |    93 | `tui/render/gauge.rs`                   | Move terminal feature. |
| `src/gimmicks/ghost.rs`                                                                                                    |    50 | `tui/input/ghost.rs`                    | Move terminal feature. |
| `src/gimmicks/mod.rs`                                                                                                      |    26 | `tui/views/gimmicks.rs`                 | Move terminal feature. |
| `src/gimmicks/packs.rs`                                                                                                    |   103 | `tui/render/packs.rs`                   | Move terminal feature. |
| `src/gimmicks/palette.rs`                                                                                                  |   103 | `tui/input/palette.rs`                  | Move terminal feature. |
| `src/gimmicks/race.rs`                                                                                                     |   129 | `tui/render/race.rs`                    | Split presentation from any session or turn decisions. |
| `src/gimmicks/savings.rs`                                                                                                  |   124 | `tui/render/savings.rs`                 | Split presentation from any session or turn decisions. |
| `src/gimmicks/session_title.rs`                                                                                            |    82 | `tui/render/session_title.rs`           | Split presentation from any session or turn decisions. |
| `src/gimmicks/shortcuts.rs`                                                                                                |   131 | `tui/input/shortcuts.rs`                | Move terminal feature. |
| `src/gimmicks/suggest.rs`                                                                                                  |   154 | `tui/input/suggest.rs`                  | Move terminal feature. |
| `src/gimmicks/title.rs`                                                                                                    |   100 | `tui/tty/title.rs`                      | Move terminal feature. |
| `src/gitflow.rs`                                                                                                           |   481 | `tui/repl/gitflow.rs`                   | Move command prompts and interaction. |
| `src/guard_cmd.rs`                                                                                                         |   172 | `tui/repl/guard_commands.rs`            | Split command parsing and terminal output from guard state changes in turn. |
| `src/headless.rs`                                                                                                          |   158 | `tui/app/headless_output.rs`            | Split JSON output wire shape from turn facts and session execution. |
| `src/host.rs`                                                                                                              |   541 | `tui/app/host.rs`                       | No domain context fits host integration; tui is closest for session commands. Split command selection from process and clipboard IO. |
| `src/layers.rs`                                                                                                            |   302 | `tui/app/layered_settings.rs`           | No domain context fits layered settings; tui is closest for session configuration. Split precedence rules from file IO and terminal formatting. |
| `src/learn.rs`                                                                                                             |   257 | `tui/views/learn.rs`                    | Move help and learning screens. |
| `src/libcmd.rs`                                                                                                            |    97 | `tui/repl/library_command.rs`           | Move session-library command parsing. |
| `src/perf.rs`                                                                                                              |   212 | `tui/app/startup_timeline.rs`           | Split timeline values from clock reads and report formatting. |
| `src/picker.rs`                                                                                                            |   435 | `tui/views/session_picker.rs`           | Move session picker and presentation. |
| `src/prefs.rs`                                                                                                             |   106 | `tui/app/preferences.rs`                | Split preference values from state-file IO; approval retains permission mode. |
| `src/projcmd.rs`                                                                                                           |    72 | `tui/repl/project_command.rs`           | No domain context fits project tooling; tui is closest for command parsing. |
| `src/repl/cache_cmds.rs`                                                                                                   |    31 | `tui/repl/cache_cmds.rs`                | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/diff_cmds.rs`                                                                                                    |   152 | `tui/repl/diff_cmds.rs`                 | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/ext_cmds.rs`                                                                                                     |   162 | `tui/repl/ext_cmds.rs`                  | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/headless.rs`                                                                                                     |    66 | `tui/repl/headless.rs`                  | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/library_cmds.rs`                                                                                                 |   291 | `tui/repl/library_cmds.rs`              | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/project_cmds.rs`                                                                                                 |   200 | `tui/repl/project_cmds.rs`              | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/session_cmds.rs`                                                                                                 |   585 | `tui/repl/session_cmds.rs`              | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/team_cmds.rs`                                                                                                    |   336 | `tui/repl/team_cmds.rs`                 | Move command delivery; keep domain rules and IO in their owning contexts. |
| `src/repl/tests/project_tests.rs`                                                                                          |   189 | `tui/repl/project_tests.rs`             | Move REPL test with the interface. |
| `src/selfupdate.rs`                                                                                                        |   317 | `tui/app/self_update.rs`                | No domain context fits self-update; tui is closest for command delivery. Split update decisions and confirmation from git and process IO. |
| `src/termenv.rs`                                                                                                           |   187 | `tui/tty/terminal_environment.rs`       | Move terminal environment detection. |
| `src/termsetup.rs`                                                                                                         |   246 | `tui/tty/terminal_setup.rs`             | Split terminal setup choices and snippets from file IO. |
| `src/tui/app.rs`                                                                                                           |  1090 | `tui/app/screen_state.rs`               | Split input actions, approval cards and state transitions into focused files. |
| `src/tui/editor.rs`                                                                                                        |   453 | `tui/input/editor.rs`                   | Move input text and edit rules. |
| `src/tui/history.rs`                                                                                                       |   193 | `tui/input/history.rs`                  | Split history navigation from history-file IO; merge storage with existing history_file.rs. |
| `src/tui/menu.rs`                                                                                                          |   287 | `tui/input/menu.rs`                     | Split menu selection from filesystem path discovery. |
| `src/tui/mod.rs`                                                                                                           |    16 | `tui/app/frontend.rs`                   | Merge frontend module declarations into the tui crate root. |
| `src/tui/run.rs`                                                                                                           |   596 | `tui/app/frontend_loop.rs`              | Split session coordination from input and terminal event handling. |
| `src/tui/screen.rs`                                                                                                        |   307 | `tui/tty/frontend_screen.rs`            | Split terminal writes from screen layout values. |
| `src/tui/transcript.rs`                                                                                                    |   120 | `tui/views/transcript.rs`               | Move scrollback presentation state. |
| `src/tui/view.rs`                                                                                                          |   834 | `tui/views/frontend_view.rs`            | Split header, input, approval and footer rendering by screen responsibility. |
| `src/typeahead.rs`                                                                                                         |   451 | `tui/input/typeahead.rs`                | Split pure key and draft rules from terminal reader thread. |
| `src/uifmt/ansi.rs`                                                                                                        |   146 | `tui/render/ansi.rs`                    | Move formatter. |
| `src/uifmt/changes.rs`                                                                                                     |   278 | `tui/render/changes.rs`                 | Move formatter. |
| `src/uifmt/diffview.rs`                                                                                                    |   562 | `tui/render/diffview.rs`                | Move formatter. |
| `src/uifmt/failure.rs`                                                                                                     |   382 | `tui/render/failure.rs`                 | Move formatter. |
| `src/uifmt/highlight.rs`                                                                                                   |   457 | `tui/render/highlight.rs`               | Move formatter. |
| `src/uifmt/identity.rs`                                                                                                    |   603 | `tui/render/identity.rs`                | Move formatter. |
| `src/uifmt/markdown.rs`                                                                                                    |   662 | `tui/render/markdown.rs`                | Move formatter. |
| `src/uifmt/permissions.rs`                                                                                                 |   360 | `tui/render/permissions.rs`             | Move formatter. |
| `src/uifmt/provider_view.rs`                                                                                               |   216 | `tui/render/provider_view.rs`           | Move formatter. |
| `src/uifmt/roles.rs`                                                                                                       |   665 | `tui/render/roles.rs`                   | Move formatter. |
| `src/uifmt/route.rs`                                                                                                       |   260 | `tui/render/route.rs`                   | Move formatter. |
| `src/uifmt/screens.rs`                                                                                                     |   337 | `tui/render/screens.rs`                 | Move formatter. |
| `src/uifmt/session.rs`                                                                                                     |   357 | `tui/render/session.rs`                 | Move formatter. |
| `src/uifmt/status.rs`                                                                                                      |   174 | `tui/render/status.rs`                  | Move formatter. |
| `src/uifmt/style.rs`                                                                                                       |   325 | `tui/render/style.rs`                   | Move formatter. |
| `src/uifmt/summary.rs`                                                                                                     |   174 | `tui/render/summary.rs`                 | Move formatter. |
| `src/uifmt/text.rs`                                                                                                        |   135 | `tui/render/text.rs`                    | Move formatter. |
| `src/uifmt/width.rs`                                                                                                       |   261 | `tui/render/width.rs`                   | Move formatter. |
| `src/uifmt/workflows.rs`                                                                                                   |   153 | `tui/render/workflows.rs`               | Move formatter. |
| `src/uifmt/wrap.rs`                                                                                                        |   164 | `tui/render/wrap.rs`                    | Move formatter. |
| `src/workstation/access.rs`                                                                                                |    44 | `tui/tty/accessibility.rs`              | Move terminal accessibility settings. |
| `src/workstation/collapse.rs`                                                                                              |    32 | `tui/render/collapse.rs`                | Move tool-output folding. |
| `src/workstation/fuzzy.rs`                                                                                                 |    71 | `tui/input/fuzzy.rs`                    | Move fuzzy input matching. |
| `src/workstation/keymap.rs`                                                                                                |   173 | `tui/input/keymap.rs`                   | Split keybinding parsing from configuration-file IO. |
| `src/workstation/mod.rs`                                                                                                   |    10 | `tui/input/workstation.rs`              | Split workstation input, rendering and terminal settings into their respective tui folders. |
| `src/workstation/vim.rs`                                                                                                   |   199 | `tui/input/vim.rs`                      | Move input mode and edits. |
| `src/testkit/capture.rs`                                                                                                   |    27 | `testkit/capture.rs`                    | Move context-free test helper to testkit. |
| `src/testkit/fake_jev.rs`                                                                                                  |    66 | `testkit/fake_jev.rs`                   | Move context-free test helper to testkit. |
| `src/testkit/fixture.rs`                                                                                                   |     9 | `testkit/fixture.rs`                    | Move context-free test helper to testkit. |
| `src/testkit/git_repo.rs`                                                                                                  |    31 | `testkit/git_repo.rs`                   | Move context-free test helper to testkit. |
| `src/testkit/golden.rs`                                                                                                    |    43 | `testkit/golden.rs`                     | Move context-free test helper to testkit. |
| `src/testkit/mod.rs`                                                                                                       |    11 | `testkit/mod.rs`                        | Move context-free test helper to testkit. |
| `src/testkit/scratch_dir.rs`                                                                                               |    19 | `testkit/scratch_dir.rs`                | Move context-free test helper to testkit. |
| `tests/fixtures/**` (26 files)                                                                                              |       | `tests/fixtures/**`                           | Not moved                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `scripts/*` (10 files)                                                                                                      |       | `scripts/*`                                   | Not moved, not edited, except the docstring of `test_provider_eval.py` (3.5) and line 23 of `provider_eval_README.md` (a path). No script reads a file under `src/`                                                                                                                                                                                                                                                           |
| `README.md`                                                                                                                 |       | `README.md`                                   | Edited in P5-6 only: the "Files" section and the one other sentence that names a source file (`src/doctor.rs`, line 334)                                                                                                                                                                                                                                                                                                      |

---

## 5. Type strictness

The user's rule: "safe typing, no any at all, or never, or unknown. All types must be defined.
Strictest codebase ever." In Rust terms, seven policies.

### 5.1 The policies

| #   | Policy                  | Rust meaning                                                                                                                                                                                                                                                                                        |
| --- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| T1  | No untyped data         | `serde_json::Value`, `serde_json::Map` and `json!` do not appear in production code outside the allow-listed wire files of 5.2, and never in any function signature or struct field that another file can see. Every external payload is deserialised at its adapter into a defined struct or enum. |
| T2  | No stringly-typed state | A closed set is an `enum`. An identifier is a newtype (`ModelId`, `SessionId`, `WorkerId`, `PermissionRule`, `ToolName`). `Result<_, String>` is replaced by the context's `Error`.                                                                                                                 |
| T3  | No panics in production | No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, no slice indexing that can panic, outside `#[cfg(test)]`.                                                                                                                                                               |
| T4  | No dynamic typing       | No `dyn Any`, no downcast, no `Box<dyn Error>`, no `anyhow::Error` in the library. Closure aliases that act as ports become named traits.                                                                                                                                                           |
| T5  | `unsafe` in one file    | One audited module with a `// SAFETY:` comment per block; `deny(unsafe_code)` everywhere else.                                                                                                                                                                                                      |
| T6  | Lossless numbers        | No `as` casts that can truncate, wrap or lose sign; `From`/`TryFrom` instead. Probabilities are `Probability`, times are `UnixMillis`.                                                                                                                                                              |
| T7  | Warnings are errors     | `-D warnings` for rustc and clippy (pedantic, with a short reviewed allow list) in the gate.                                                                                                                                                                                                        |

### 5.2 Where untyped JSON is still allowed, and why

Three situations cannot be fully typed without changing behaviour, because the program must
pass through data whose shape it does not own (decision U3: accepted). Each gets one private
wire file. `Value` there is a private field or local variable; the file's public functions take
and return defined types.

Why typed structs cannot replace them, checked against the code and the dependency features:

| Alternative                                                     | Why it changes bytes or does not remove `Value`                                                                                                                                                                                                                               |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A struct with `#[serde(flatten)] rest: BTreeMap<String, Value>` | Still a `Value`. Known fields are written first and the rest after, while today's output is one object with all keys in byte order (`serde_json` is built without `preserve_order`: `cargo tree -e features` shows only `raw_value`, and that one only through another crate) |
| `Box<RawValue>` for unknown parts                               | `RawValue` cannot be used under `flatten`, and it keeps the sender's bytes, while today's code re-serialises: keys sorted, whitespace dropped, numbers normalised (`1e2` becomes `100.0`). Keeping the original bytes would be a behaviour change                             |
| Typing every field                                              | The fields are not ours: Claude Code adds settings keys and the Anthropic API adds request parameters without notice                                                                                                                                                          |

Each of the three files is held to the same limits (check A6): no `pub` item whose signature
names `Value` or `Map`; a round-trip unit test with unknown keys, nested objects, a key order
that is not sorted and the number forms above, compared with a golden; under 150 lines.

| Allow-listed file                                  | Why `Value` is needed                                                                                                                                                                                                                                                                                                                                                                  | Typed surface the rest of the program sees                                                                                             |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `approval/infrastructure/claude_settings_wire.rs`  | `.claude/settings.local.json` belongs to Claude Code. Keys we do not know must survive a rewrite, in the same order and form as today.                                                                                                                                                                                                                                                 | `PermissionSettings { allow: Vec<PermissionRule>, directories: Vec<PathBuf> }`, `add_allow_rule`, `remove_allow_rule`, `add_directory` |
| `gateway/infrastructure/anthropic_wire.rs`         | The gateway forwards requests with fields it does not know (new API parameters) and must only replace `model` and adapt a few fields for Haiku. `set_model` (`bin/gateway.rs:849`) first swaps the model inside the original bytes and re-serialises through `Value` only when that fails; `adapt_for_haiku` (`:827`) always re-serialises. Both paths are kept and each has a golden. | `MessagesRequest` view: `model`, `messages: Vec<WireMessage>`, `system`, `tools`, `metadata`; `with_model(ModelId) -> Vec<u8>`         |
| `backend/infrastructure/claude/permission_wire.rs` | A tool's input is echoed back to the CLI as `updatedInput`. Today it is re-serialised from a `Value`, which sorts keys. To keep the bytes identical, the echo is canonicalised once at this boundary into `OpaqueJson`.                                                                                                                                                                | `ToolInput` (typed, for review and rules) plus `OpaqueJson` (a validated JSON text with a redacting `Debug`, never inspected)          |

`gateway/domain/prefix_fingerprint.rs` hashes arbitrary request blocks today (`prefix.rs:39`):
it walks the `Value`, so keys are visited in byte order, `cache_control` is skipped at every
depth and whitespace does not count. The walk moves unchanged into `anthropic_wire.rs`; the
domain sees only `PrefixFingerprint` (hashes and lengths). Hashing the sender's raw block text
instead would report a cache break whenever the client moves a `cache_control` marker or
reorders a key, which changes the `prefix` field of the usage log. A golden pins that field
for two requests that differ only in those two ways.

The allow list is a `const` in `tests/architecture.rs` (check A6). Adding a fourth file needs a
change to that test, which is a reviewed decision, not a drive-by.

### 5.3 How each payload becomes typed without changing behaviour

`Value` probing is lenient: a missing field is `null`, an unknown event type is ignored. A
derived `Deserialize` is strict by default. To keep behaviour, every wire type follows these
rules, and every conversion is proved by a differential test before the old code is deleted.

- Events are `#[serde(tag = "type")]` enums with an `#[serde(other)] Unknown` variant;
  unknown fields are ignored (never `deny_unknown_fields`).
- A derived `Option<String>` fails the whole line when the field holds a number, where
  `v["x"].as_str()` gives `None` and the rest of the line is still used. Every field is
  therefore `#[serde(default, deserialize_with = "lenient::...")]`, from one file
  `backend/infrastructure/lenient.rs` (string, unsigned, float, bool, array, object: each
  yields the default on a wrong type, as `as_str`, `as_u64`, `as_f64` do today).
- A line whose `type` is missing or not a string must give what it gives today. The adapter
  reads the tag leniently first and maps both cases to `Unknown`.
- Number readers keep today's tolerance (`count(v, key)` in `claude.rs:134`, `codex.rs:145`,
  `gemini.rs:143`, `grok.rs:54` accept what they accept today) through one shared
  `deserialize_with` function in the adapter folder.
- **Differential oracle.** In the commit that introduces a typed parser, the old `Value`
  parser moves into the test module unchanged. A test feeds both parsers the same input and
  requires identical `Vec<Event>` and identical final result (`Finish`, usage, session id).
  The input is every file in `tests/fixtures/<cli>/`, fed whole and in order because Codex and
  Gemini keep state between lines (`backend/codex.rs:70`, `backend/gemini.rs:102`), and then
  the same files with one mutation at a time, generated by a loop with no randomness. For
  every key at every depth of every line: remove it; replace its value with `null`, `0`, `1.5`,
  `"x"`, `"5"`, `true`, `[]`, `{}`, `-1`, `2.0`, `1e2`, `18446744073709551616`; add the same key
  a second time with a different value. For every line: unknown `type`, missing `type`, numeric `type`,
  the line cut at each quarter, an empty line, a line that is not JSON. A mismatch prints the
  mutated line. The oracle stays for one commit; the next commit deletes it and the old parser.
- **Output bytes.** `serde_json` is built without `preserve_order` here, so every `json!`
  object is written with its keys in byte order, at every depth. A derived `Serialize` writes
  fields in declaration order. The rules that keep the bytes:

| Trap                            | Rule                                                                                                                                                                                                                                                     |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Key order                       | Fields are declared in byte order of their serialised names (`error` before `error_kind`, `session` before `session_id`, capitals before lower case). A unit test per row struct serialises it, reads the keys back and asserts they are sorted          |
| `#[serde(flatten)]`             | Not used in a row struct: it writes the inner keys in one block and cannot interleave them (D18)                                                                                                                                                         |
| Absent or `null`                | `Option::None` is written as `null`, as `json!` does. `skip_serializing_if` only where today's code inserts the key conditionally: `failover_notes`, `cancelled` (`turn.rs:1094-1100`)                                                                   |
| A key written twice             | `error_kind` is overwritten with `"cancelled"` today (`turn.rs:1095`); the typed row computes the one value                                                                                                                                              |
| Numbers                         | Counts stay `u64` and probabilities stay `f64` in the row structs; no `f32` exists today and none is introduced (an `f32` prints different digits). A non-finite `f64` is `null` both ways                                                               |
| Nested objects                  | `signals`, `usage`, `plan` are row structs under the same rules                                                                                                                                                                                          |
| Maps                            | A map field of a DTO is a `BTreeMap<String, _>`, never a `HashMap`: `Value` writes keys in byte order, a `HashMap` in a different order on every run (cooldowns by provider, session ids by provider)                                                    |
| Compact or pretty               | Each writer keeps the call it uses today (`to_string`, `to_string_pretty`, `to_vec`); the golden holds the whole file, newline at the end included                                                                                                       |
| A key twice in one input object | `Value` keeps the last one and uses the line; a derived `Deserialize` rejects the line (`duplicate field`). The oracle has this mutation. A wire struct that fails it gets a hand-written map visitor in which a later key overwrites, from `lenient.rs` |
| Text                            | Strings go through the same `serde_json` escaper either way                                                                                                                                                                                              |

The goldens of 4.1 are written from today's code in P0-3, **before** any conversion, with at
least two cases per format: every optional key present, and every optional key absent.

| Payload                  | Today (untyped)                                                                                                                                                                                                                                                                 | Typed replacement                                                                                                                                                                                                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Jev request              | `state: Value, questions: Value` (`typesafe.rs:58-112`), 8 question builders and 3 state builders returning `Value` (`router.rs:528-693`)                                                                                                                                       | `JudgmentState { request, recent_context, previous: Option<PreviousTurn> }`, `Question { id: QuestionId, kind: Choice(Vec<Criterion>) \| Noul(TrueFalse), instructions }`; serialised by `typesafe_wire.rs`                                                         |
| Jev response             | `ask* -> Result<Value>`, `decode_choice(&Value, key)`, `decode_noul` (`typesafe.rs:132-138`), `decode_provider_want(&Value, ...)` (`router.rs:704`)                                                                                                                             | `Answers` (map `QuestionId -> Answer::{Choice(ChoiceAnswer), Noul(Probability)}`) deserialised in `typesafe_wire.rs`                                                                                                                                                |
| Signals for the log      | `pub signals: Value` in `CodeWant` (`router.rs:27`) and `ProviderWant` (`router.rs:53`); `Value::Null` for "not asked" (`router.rs:462`); `Judged::log_signals -> Value` (`turn.rs:289`); `jev: &Value` (`turn.rs:1081`, `delegate.rs:1356`, `delegate.rs:1373`)                | `Signals` struct in `judgment/domain/signals.rs`; "not asked" is `Option<Signals>`; `SignalsRow` in the log wire files writes the same keys                                                                                                                         |
| Signals read back by key | `worker_alias(tier: usize, signals: &Value, ...)` reads `signals["edit_p"]`, `["confidence"]`, `["choice"]` (`router.rs:225-235`); `worker_want -> Option<(&'static str, String, Value)>` (`router.rs:264`)                                                                     | `fn worker_alias(want: &TierWant, ...) -> WorkerAlias` (enum `Haiku \| Sonnet \| Opus`); `worker_want -> Option<WorkerJudgment>`                                                                                                                                    |
| Orchestration fields     | `Orchestration::fields -> Value` (`router.rs:121`)                                                                                                                                                                                                                              | `OrchestrationRow` in `gateway/infrastructure/decision_log_wire.rs`                                                                                                                                                                                                 |
| Claude stream            | 10 functions over `&Value` (`backend/claude.rs:97-181`)                                                                                                                                                                                                                         | `ClaudeLine` enum in `claude_wire.rs`                                                                                                                                                                                                                               |
| Codex stream             | 6 functions over `&Value` (`backend/codex.rs:120-182`)                                                                                                                                                                                                                          | `CodexLine` enum in `codex_wire.rs`                                                                                                                                                                                                                                 |
| Gemini and agy streams   | 9 functions over `&Value` (`backend/gemini.rs:143-282`)                                                                                                                                                                                                                         | `GeminiLine`, `AgyLine`                                                                                                                                                                                                                                             |
| Grok stream              | 5 functions over `&Value` (`backend/grok.rs:54-103`)                                                                                                                                                                                                                            | `GrokLine`                                                                                                                                                                                                                                                          |
| Tool input               | `PermissionRequest.input: serde_json::Value` (`backend/mod.rs:150`), read by about 20 functions in `permissions.rs:41-550`                                                                                                                                                      | `ToolInput` enum: `Bash { command }`, `Write { path, content }`, `Edit { path, old, new }`, `MultiEdit { path, edits }`, `NotebookEdit { .. }`, `WebFetch { url }`, `Other { fields: Vec<ToolField> }` (each unknown field already rendered to text at the adapter) |
| Tool call summary        | `Event::Tool(String)` (`backend/mod.rs:123`), parsed back by splitting on whitespace in `handoff.rs:225-250`                                                                                                                                                                    | `Event::Tool(ToolCall { name: ToolName, argument: String })`; `session::tool_trail` reads the fields; the display string is built by `tui/render`                                                                                                                   |
| Turn log row             | `turn_row(..., signals: &Value, ...) -> Value` (`turnlog.rs:38-46`), `usage_json`, `plan_json`                                                                                                                                                                                  | `TurnLogRow`, `UsageRow`, `PlanRow` (`turn_log_wire.rs`)                                                                                                                                                                                                            |
| `jev-ask` output         | `ask_json -> Value` (`turnlog.rs:69`)                                                                                                                                                                                                                                           | `AskOutput` (`backend/interface/ask_output.rs`)                                                                                                                                                                                                                     |
| Log writer               | `log::append(path, &Value)` (`log.rs:48`)                                                                                                                                                                                                                                       | `jsonl_log::append<T: Serialize>(path, &T)`                                                                                                                                                                                                                         |
| Saved session            | `Sessions::to_json/from_json(&Value)` (`handoff.rs:359-382`), `SavedSession::to_json/from_json` (`handoff.rs:469-480`), `read_session` (`handoff.rs:572`)                                                                                                                       | `SavedSessionFile` DTO in `session_wire.rs` with the same leniency as `from_json` today                                                                                                                                                                             |
| Cooldown file            | `Cooldowns::to_json/from_json(&Value)` (`cooldown.rs:314-320`)                                                                                                                                                                                                                  | `CooldownFile` DTO                                                                                                                                                                                                                                                  |
| Chain overrides          | `let value: Value = serde_json::from_str(json)` (`catalog.rs:271`)                                                                                                                                                                                                              | `ChainsFile` DTO; error text kept                                                                                                                                                                                                                                   |
| Codex auth token         | two `Value` parses (`availability.rs:387-389`)                                                                                                                                                                                                                                  | `CodexAuthFile`, `CodexIdClaims` DTOs                                                                                                                                                                                                                               |
| Worker record            | `Saved` built with `json!` (`delegate.rs:676-733`)                                                                                                                                                                                                                              | `WorkerFile` DTO                                                                                                                                                                                                                                                    |
| MCP protocol             | 22 uses of `Value` in `bin/delegate_mcp.rs` (`tools_list`, `parse_delegate`, `rpc_result`, `handle`)                                                                                                                                                                            | `JsonRpcRequest`, `JsonRpcResponse`, `ToolCallParams`, `DelegateArguments`, `ShowDiffArguments` in `json_rpc.rs`, `tool_arguments.rs`. The tool input schemas are static JSON text checked by a test                                                                |
| Gateway usage            | `UsageTap.record: Value` (`usage.rs:113`), `set(key, Value)` (`usage.rs:147`), `error: Option<Value>` (`usage.rs:106`), SSE handling over `&Value` (`usage.rs:194-213`), `decision: Value` (`usage.rs:335`, `usage.rs:457`), `finish_check(.., Option<Value>)` (`usage.rs:472`) | `UsageRecord`, `SseEvent` enum, `ModelChoiceRecord`, `CheckResult` enum                                                                                                                                                                                             |
| Gateway request reading  | 47 uses in `bin/gateway.rs` (`choose_model(.., req: &Value)` line 565, `last_user_prompt` 712, `recent_context` 801, `adapt_for_haiku(&mut Value)` 827, `set_model` 849)                                                                                                        | `MessagesRequest` view from `anthropic_wire.rs`; pure rules in `gateway/domain` take `&[WireMessage]`                                                                                                                                                               |

### 5.4 Stringly-typed states and loose signatures found

| Where                                                                     | Today                                                                                                                                                                                              | Becomes                                                                                                                        |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `router.rs:14-19`                                                         | `Decision { model: &'static str, kind: String, confidence: f64, reason: &'static str }`                                                                                                            | `TierRoute { tier: LegacyTier, kind: RequestKind, confidence: Probability, reason: TierReason }` in `judgment`                 |
| `router.rs:23`, `orchestrate.rs:45`, `orchestrate.rs:57`, `router.rs:225` | tier as `usize`, indexing `TIERS` arrays (`bin/gateway.rs:51`, `bin/route.rs:38`, `turn.rs:45`)                                                                                                    | `LegacyTier` enum with `model_id()`                                                                                            |
| `anthropic.rs:23`                                                         | `Message.role: String`                                                                                                                                                                             | `Role` enum                                                                                                                    |
| `backend/mod.rs:106`                                                      | `PlanUsage.window: String`                                                                                                                                                                         | `PlanWindow` enum with `Other(String)` for a window name the CLI invents                                                       |
| `backend/mod.rs:203`, `handoff.rs:22`, `turn.rs:256`, `delegate.rs:892`   | model as `String`                                                                                                                                                                                  | `ModelId`                                                                                                                      |
| `backend/mod.rs:86-88`                                                    | `ErrorKind::Quota { resets_at: Option<String> }`                                                                                                                                                   | keeps the CLI's wording as `ResetText` (newtype); parsed by `providers::reset_time`                                            |
| `turn.rs:49-97`                                                           | `Ui::route(&str)`, `note(&str)`, `no_route(.., text: &str)`, `cancelled(text: &str)`: the engine formats text for the screen                                                                       | `TurnObserver` receives typed facts (`RouteDecision`, `Note` enum, `NoRouteInfo`, `CancelInfo`); `tui/render` writes the words |
| `turn.rs:298-300`, `delegate.rs:978-979`                                  | `BackendFactory`, `GitStatus`, `Clock`, `Judge` as `Box<dyn Fn ...>` aliases                                                                                                                       | named traits in `ports.rs`                                                                                                     |
| 34 signatures                                                             | `Result<_, String>` in `delegate.rs` (12), `bin/delegate_mcp.rs` (10), `catalog.rs` (3), `permissions.rs` (3), `bin/code.rs` (2), `backend/mod.rs`, `handoff.rs`, `repl.rs`, `routing.rs` (1 each) | the context's `Error` enum; the text shown to the user is produced by `Display` and pinned by tests                            |
| `router.rs`, `typesafe.rs`, `delegate.rs`, `anthropic.rs`                 | `anyhow::Result` in four library files                                                                                                                                                             | `judgment::Error`, `orchestration::Error`, `chat::Error`                                                                       |
| `usage.rs:147`, `usage.rs:472`                                            | record fields set by string key                                                                                                                                                                    | fields of `UsageRecord`                                                                                                        |

### 5.5 Panics, `unsafe`, downcasts found

Production code today has 11 panic sites, `unsafe` in 3 files, and 1 downcast.

| Where                        | Today                                                             | Fix                                                                                                                        |
| ---------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `catalog.rs:180`             | `panic!("catalog chain names unknown model ...")`                 | Chains name `&'static ModelSpec` constants instead of ids, so an unknown name is a compile error                           |
| `turn.rs:1142`               | `.expect("legacy tiers are catalog models")`                      | `LegacyTier::spec()` returns the constant                                                                                  |
| `handoff.rs:541`             | `.expect("an unbounded range")`                                   | a `loop` that returns                                                                                                      |
| `permissions.rs:666-669`     | three `.expect("checked in parse")` on a `Value`                  | the typed settings view owns the lists                                                                                     |
| `usage.rs:522`               | `.expect("checked")`                                              | `if let Some(open) = ...`                                                                                                  |
| `prefix.rs:125`              | `.lock().unwrap()`                                                | `shared::lock`                                                                                                             |
| `bin/gateway.rs:297`, `:607` | `.unwrap()` on a response builder and on a checked `Option`       | a fallible builder mapped to a fixed 502 response; `let Some(prompt) = ... else`                                           |
| `bin/route.rs:284`           | two `.expect("object")` on `Value`                                | a typed output row                                                                                                         |
| `backend/mod.rs:877`, `:945` | `unsafe` `killpg` and `kill(pid, 0)`                              | `sys_unix::signal_process_group`, `sys_unix::process_exists`                                                               |
| `lineedit.rs:318-384`        | `unsafe` `poll`, `read`, `tcgetattr`, `tcsetattr` (8 blocks)      | `sys_unix::RawMode` guard, `sys_unix::read_byte`                                                                           |
| `terminal.rs:36-136`         | `unsafe` termios save and restore, `ioctl(TIOCGWINSZ)` (6 blocks) | `sys_unix::saved_termios`, `restore_termios`, `set_echo`, `window_size`                                                    |
| `backend/mod.rs:1173`        | `unsafe { libc::kill(child, SIGKILL) }` in a test                 | `sys_unix::signal_process`: `forbid(unsafe_code)` holds in test code too, and `libc` leaves `backend`'s manifest           |
| `terminal.rs:169-171`        | `payload().downcast_ref::<&str>()` and `::<String>()`             | `PanicHookInfo::payload_as_str()` (stable since Rust 1.81; the toolchain here is 1.95.0), same result for both payload kinds |

`shared/infrastructure/sys_unix.rs` is the only file with `unsafe`. Each block gets a
`// SAFETY:` comment; `clippy::undocumented_unsafe_blocks` is denied. Every crate root except
`shared`'s, and every binary, has `#![forbid(unsafe_code)]`; `shared` has `deny` from the
workspace table and one `#[expect(unsafe_code, reason = "...")]` on that module (check A7).

The audit's "2 hits" for `unsafe` in `router.rs` are the word in a security keyword list, not
code.

44 `as` numeric casts exist in production code; each becomes `From`, `TryFrom` with a typed
error, or a named saturating conversion in `UnixMillis`/`TokenUsage`.

### 5.6 Lint configuration

The root `Cargo.toml` holds the tables once; every member opts in with

```toml
[lints]
workspace = true
```

(check C1). Tried on this toolchain in a scratch workspace: every lint name below exists in
clippy (as of `296bd5b`; recheck with clippy 0.1.95), a member that opts in is held to the tables, and a `[workspace.lints]` table
that no package opts into is silently ignored, which is why C1 exists. `clippy.toml` is read
from the workspace root for every member.

```toml
[workspace.lints.rust]
unsafe_code = "deny"                # crate roots add forbid, except shared
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
exit = "deny"                    # only the binaries may exit
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

What clippy cannot see, and what covers it:

| Gap (tried in the scratch crate)                                                                                                                                                                                                           | Covered by                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `disallowed_types` on `anyhow::Error` does not fire on `anyhow::Result<T>` written as an alias, so it is not in `clippy.toml`                                                                                                              | `anyhow` is a dependency of `crates/cli` only, so no library crate can name it (Cargo, check C1)                                                             |
| `allow-unwrap-in-tests` and its siblings cover `#[test]` functions and `#[cfg(test)]` modules, but `as_conversions`, `string_slice`, `print_stdout`, `wildcard_enum_match_arm`, `too_many_lines` and `unreachable_pub` have no test switch | each `mod tests` item carries one fixed line, `#[allow(<those six>, reason = "test code")]`; `testkit` and `testing` modules carry the same line (check A14) |
| the gate lints test code too (`--all-targets`), and the 866 below counts production code only                                                                                                                                              | `pedantic` stays on in tests: about 90 more findings, fixed in the same P4-12 commits                                                                        |
| a `Value` hidden behind a type alias                                                                                                                                                                                                       | check A6 scans tokens, not types                                                                                                                             |

**What turning the tables on costs.** Measured with the lints as warnings on today's production
code (library and binaries, tests excluded): 866 findings.

| Group                                            | Findings | Nature                                                                                               |
| ------------------------------------------------ | -------: | ---------------------------------------------------------------------------------------------------- |
| `must_use_candidate`, `return_self_not_must_use` |      280 | add `#[must_use]`; no behaviour                                                                      |
| `indexing_slicing`, `string_slice`               |      147 | each becomes `get`/`split_at_checked`; these are the real panic risks and the reason P4-11 is risk M |
| `as_conversions` and the `cast_*` lints          |       93 | `From`/`TryFrom` (5.5 counted 44 cast sites; several lints fire per site)                            |
| `print_stdout`, `print_stderr`                   |       61 | move behind `tui` console and the binaries' output files                                             |
| `doc_markdown`                                   |       59 | backticks in comments                                                                                |
| `missing_debug_implementations`                  |       32 | derive or hand-write (redacting where a field is a `Secret`)                                         |
| `wildcard_enum_match_arm`                        |       25 | name the variants                                                                                    |
| everything else in `pedantic`                    |      169 | mostly mechanical                                                                                    |

These are fixed lint group by lint group in P4-12, one commit per group and crate, never mixed
with a move.

CI is section 3.6.

---

## 6. SOLID in the target structure

| Principle             | How the structure provides it                                                                                                                                                                                                                                                                                                                                  |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Single responsibility | One concept per file; a README purpose without "and"; size budgets force the split when a second responsibility grows.                                                                                                                                                                                                                                         |
| Open/closed           | A new provider is a new folder under `backend/infrastructure/` and `providers/infrastructure/probes/`, a row in `model_table.rs`, and one `ProviderId` variant; `wildcard_enum_match_arm` makes the compiler list every `match` to extend. No existing rule is edited.                                                                                         |
| Liskov                | Every backend passes the same contract tests (`backend/infrastructure/contract_tests.rs`: scripted CLI, same assertions for all five). Fakes in each crate's `testing` module implement the same ports.                                                                                                                                                        |
| Interface segregation | Small ports per use case (`TurnRunner`, `Approver`, `WorkingTree`, `SessionStore`, `CooldownStore`, `JudgmentService`, `Clock`, `Env`). `Engine` today takes 11 builder options and reaches into `Runtime`'s public fields; in the target it holds ports. `Ui` (9 methods, seven for the screen, two for approvals) splits into `TurnObserver` and `Approver`. |
| Dependency inversion  | Use cases name traits from `application/ports.rs`; adapters implement them; the binaries in `crates/cli` and `compose.rs` files are the only places that name a concrete adapter.                                                                                                                                                                              |

---

## 7. SOLID and DRY audit of the current code

### 7.1 Layering and responsibility violations

| #   | Where                                                                                                                                                                                                                                                                                                                                                                        | Finding                                                                                                                                                                                                                                    | Removed by                                                                                                                                                                                                                                                                                                                |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| V1  | `turn.rs:354-389`, `:390-1139`                                                                                                                                                                                                                                                                                                                                               | `Engine` is routing state, session store, change tracker, orchestration planner, logger, approver and failover loop in one 750-line `impl` with 50 methods.                                                                                | Split into the files of `turn/application/` (7.3)                                                                                                                                                                                                                                                                         |
| V2  | `delegate.rs:1-1643`                                                                                                                                                                                                                                                                                                                                                         | Worker policy, patch parsing, git worktrees, a persisted record store, process liveness, crash recovery and the delegation use cases in one file.                                                                                          | `orchestration/` layers (7.3)                                                                                                                                                                                                                                                                                             |
| V3  | `backend/mod.rs`                                                                                                                                                                                                                                                                                                                                                             | Domain types (`ProviderId`, `Event`, `ErrorKind`), the approval port, error-text heuristics, prompt temp files, and a 450-line child-process runner in one file. Every module that needs `ProviderId` imports the process runner's module. | `shared/domain/provider_id.rs`, `backend/domain`, `backend/infrastructure/process`                                                                                                                                                                                                                                        |
| V4  | `bin/gateway.rs` (1,063 lines), `bin/delegate_mcp.rs` (596), `bin/code.rs` (405)                                                                                                                                                                                                                                                                                             | Business rules and their tests live in binaries, where no other code (Flashboard) can reuse them. `forward` is 145 lines, `choose_model` 93.                                                                                               | `gateway/`, `orchestration/interface/mcp/`, `tui/app/`; binaries become roots                                                                                                                                                                                                                                             |
| V5  | `providers.rs:455-510`, `availability.rs:168-198`, `turn.rs:610-619`                                                                                                                                                                                                                                                                                                         | Screen text is produced in domain and engine code.                                                                                                                                                                                         | `tui/render/providers_text.rs`                                                                                                                                                                                                                                                                                            |
| V6  | `uifmt/failure.rs:33`, `uifmt/permissions.rs:145`, `uifmt/mod.rs` (imports `backend`)                                                                                                                                                                                                                                                                                        | The presentation module, documented as "input structs hold plain strings and numbers, never engine types", imports `doctor`, `permissions` and `backend`.                                                                                  | `tui/views` builds facts; `render` takes facts only                                                                                                                                                                                                                                                                       |
| V7  | `commands.rs:6-8`                                                                                                                                                                                                                                                                                                                                                            | A module documented as pure imports `uifmt`, `turn`, `permissions`, `backend`.                                                                                                                                                             | `tui/command.rs` depends on `command_table.rs` and context roots                                                                                                                                                                                                                                                          |
| V8  | `catalog.rs:252`, `router.rs:239`, `backend/claude.rs:60-64`, `backend/codex.rs:52-53`, `backend/grok.rs:43`, `backend/gemini.rs:114`, `providers.rs:245`, `:384`, `orchestrate.rs:135`                                                                                                                                                                                      | Constructors and pure modules read the environment themselves (hidden input, not testable without process state).                                                                                                                          | `config.rs` per context, built once from `&dyn Env`                                                                                                                                                                                                                                                                       |
| V9  | `backend/claude.rs:24-25`, `backend/mod.rs:26`                                                                                                                                                                                                                                                                                                                               | The backend layer depends on the chat client module (`anthropic`) for limit-text rules, and on `permissions`.                                                                                                                              | `backend/domain/limit_text.rs`; `approval` is a declared upstream                                                                                                                                                                                                                                                         |
| V10 | `providers.rs:181-204`                                                                                                                                                                                                                                                                                                                                                       | `Runtime` has eight `pub` fields mutated from `turn.rs`, `delegate.rs`, `repl.rs` and `views.rs`: no invariant can be kept.                                                                                                                | `ProviderRegistry` with methods only                                                                                                                                                                                                                                                                                      |
| V11 | `router.rs`                                                                                                                                                                                                                                                                                                                                                                  | One file holds the legacy three-tier router, the provider judgment, eight question texts, keyword security lists, thresholds and the worker alias rule.                                                                                    | `judgment/` split (7.3)                                                                                                                                                                                                                                                                                                   |
| V12 | `handoff.rs`                                                                                                                                                                                                                                                                                                                                                                 | Provider session ids, the turn log, catch-up rendering, the saved-session format and its file handling in one file.                                                                                                                        | `session/` split                                                                                                                                                                                                                                                                                                          |
| V13 | `doctor.rs:218-259`, `providers.rs:515`                                                                                                                                                                                                                                                                                                                                      | Provider setup knowledge (install hint, login command) is owned by `doctor` but used by `uifmt` and `providers`.                                                                                                                           | `providers/domain/setup_guide.rs`                                                                                                                                                                                                                                                                                         |
| V14 | `delegate.rs:61`                                                                                                                                                                                                                                                                                                                                                             | Orchestration imports the turn engine for an idle timeout and a factory alias, closing a cycle.                                                                                                                                            | `backend::BackendFactory` port; idle timeout in `backend` config                                                                                                                                                                                                                                                          |
| V15 | `turn.rs:49-97`                                                                                                                                                                                                                                                                                                                                                              | `Ui` mixes progress reporting and tool approval, and takes pre-formatted text.                                                                                                                                                             | `TurnObserver` + `Approver`, typed facts                                                                                                                                                                                                                                                                                  |
| V16 | 16 production files over 400 lines; 12 functions over 60 lines (`bin/gateway.rs:302` 145, `bin/route.rs:137` 119, `turn.rs:698` 109, `bin/gateway.rs:565` 93, `bin/chat.rs:37` 85, `lineedit.rs:306` 81, `usage.rs:379` 76, `delegate.rs:1369` 72, `changes.rs:714` 67, `bin/tune.rs:40` 67, `turn.rs:951` 64, `console.rs:601` 64)                                          | Size hides structure.                                                                                                                                                                                                                      | Budgets (9.2)                                                                                                                                                                                                                                                                                                             |
| V17 | `changes.rs:460-497` (`new_file_lines`, `new_file_diff`, `is_executable`), `changes.rs:672-713` (`inside_root`, `blocking_parent`, `folder_in_the_way`), `permissions.rs:535`, `:547` (`blocked_folder` asks `is_dir`, `always_plan` reads `HOME`), `handoff.rs:507` (`project_key` canonicalises), `delegate.rs:791`, `:841-878` (`has_leftovers`, `Scan` read directories) | Rules that 7.3 files under `domain/` read the disk or the environment themselves. Two of them decide what "always" may grant, a security boundary.                                                                                         | The rule takes the fact as an argument (is it a folder, the home folder, the canonical path, the directory listing); the read moves to that crate's `infrastructure/` (`git_working_tree.rs`, `undo_files.rs`, `directory_path.rs`, `session_file.rs`, `worker_file.rs`). Done in P3-6 with every existing test unchanged |

### 7.2 Duplication

| #   | What is duplicated                 | Where                                                                                                                                                                                                                                                                                                                                                                           | One home                                                                                                                                                                                                                                                                                                                                                          |
| --- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Running `git`                      | `changes.rs:245` (`git`), `changes.rs:302`, `delegate.rs:386` (`git`), `turn.rs:303` (`git_status_short`), `doctor.rs:463` (`in_git_repo`)                                                                                                                                                                                                                                      | `shared/infrastructure/git_command.rs`                                                                                                                                                                                                                                                                                                                            |
| D2  | Current time in ms                 | `log.rs:28`, `doctor.rs:543`; clock closure aliases `turn.rs:300`, `delegate.rs:979`; nanosecond stamps for unique names at `changes.rs:268`, `backend/mod.rs:430`, `delegate.rs:1119`                                                                                                                                                                                          | `Clock` port; `private_file::unique_name`                                                                                                                                                                                                                                                                                                                         |
| D3  | Write a private file atomically    | `handoff.rs:546` (`write_atomic`), `permissions.rs:711` (`write_private_atomic`), `backend/mod.rs:425` (`write_private`), `cooldown.rs:343-349`, `delegate.rs:758-764`, mode setting at `lineedit.rs:163`                                                                                                                                                                       | `shared/infrastructure/private_file.rs`                                                                                                                                                                                                                                                                                                                           |
| D4  | Parse a float from the environment | `bin/gateway.rs:105` (`env_f64`), `bin/route.rs:132` (`env_or`), `envfile.rs:54` (`lookup_f64`). The first two skip the env file and accept non-finite values, the third does not                                                                                                                                                                                               | `Env::f64(EnvVar, default)`; the difference is kept per variable and written down in `EnvVar`                                                                                                                                                                                                                                                                     |
| D5  | Idle timeout from the environment  | `turn.rs:317-326` (`CODE_IDLE_TIMEOUT_SECS`), `bin/ask.rs:21` (`ASK_IDLE_SECS`)                                                                                                                                                                                                                                                                                                 | `backend/infrastructure/config.rs`                                                                                                                                                                                                                                                                                                                                |
| D6  | Home directory                     | eight reads of `HOME`: `envfile.rs:43`, `doctor.rs:491`, `providers.rs:106`, `log.rs:14`, `availability.rs:233`, `bin/code.rs:222`, `:271`, `permissions.rs:547`                                                                                                                                                                                                                | `Env::home()`                                                                                                                                                                                                                                                                                                                                                     |
| D7  | Lock a mutex ignoring poison       | `usage.rs:368`, `providers.rs:282`, `backend/codex.rs:70`, `console.rs:186`, `backend/gemini.rs:102`, `delegate.rs:990`, `backend/mod.rs:489`, `:496`, `turn.rs:107`                                                                                                                                                                                                            | `shared/domain/lock.rs`                                                                                                                                                                                                                                                                                                                                           |
| D8  | Cut text to a length               | `handoff.rs:298` (`truncate_chars`), `handoff.rs:308` (`truncate_lines`), `delegate.rs:339` (`truncate_with_note`), `backend/mod.rs:585` (`last_chars`)                                                                                                                                                                                                                         | `shared/domain/truncation.rs` (four named functions, since their outputs differ)                                                                                                                                                                                                                                                                                  |
| D9  | "How long" in words                | `providers.rs:165` (`format_left`), `views.rs:186` (`human_ago`), `uifmt/text.rs`                                                                                                                                                                                                                                                                                               | `shared/domain/duration_text.rs`                                                                                                                                                                                                                                                                                                                                  |
| D10 | Legacy tier table                  | `bin/gateway.rs:51`, `bin/route.rs:38`, `turn.rs:45`, plus `router.rs:9` and `routing.rs:18` importing the constants                                                                                                                                                                                                                                                            | `catalog/domain/legacy_tier.rs`                                                                                                                                                                                                                                                                                                                                   |
| D11 | "Is this a harness message"        | `bin/gateway.rs:664-680` (two functions), `bin/orch_tune.rs:31` (one function with all four prefixes, shorter spellings)                                                                                                                                                                                                                                                        | `gateway/domain/harness_message.rs`, both tools call it with the same options they use today                                                                                                                                                                                                                                                                      |
| D12 | `MINUTE_MS` and time constants     | `providers.rs:31`, `cooldown.rs:18-21`                                                                                                                                                                                                                                                                                                                                          | `shared/domain/unix_millis.rs`                                                                                                                                                                                                                                                                                                                                    |
| D13 | Is this file executable            | `changes.rs:487`, `availability.rs:215`                                                                                                                                                                                                                                                                                                                                         | `shared/infrastructure/private_file.rs` neighbour `file_mode` function in `sys_unix.rs`                                                                                                                                                                                                                                                                           |
| D14 | Usable models list                 | `availability.rs:156` (`usable_models`), `views.rs:61` (`usable_models`)                                                                                                                                                                                                                                                                                                        | `providers::ProviderRegistry::usable`                                                                                                                                                                                                                                                                                                                             |
| D15 | Per-CLI JSON number reader         | `backend/claude.rs:134` (`count`), `backend/codex.rs:145` (`count`), `backend/gemini.rs:143` (`uint`), `backend/grok.rs:54` (`count`); four `usage_of` functions                                                                                                                                                                                                                | one `deserialize_with` in `backend/infrastructure/`                                                                                                                                                                                                                                                                                                               |
| D16 | Test scaffolding                   | temp dir helpers at `cooldown.rs:763`, `lineedit.rs:518`, `views.rs:474`, `permissions.rs:849`, `console.rs:1084`, `delegate.rs:1880`, `backend/mod.rs:1369`, `changes.rs:826`; fake script backends at `turn.rs:1176`, `repl.rs:433`, `delegate.rs:1825`, `backend/mod.rs:975`, `:1600`; temp git repos at `changes.rs:859`, `repl.rs:500`, `turn.rs:2122`, `delegate.rs:2161` | `crates/testkit/` and each crate's `testing` module (3.3)                                                                                                                                                                                                                                                                                                         |
| D17 | Tool summary formatted then parsed | written as `"Name args"` by each backend, split again in `handoff.rs:225`                                                                                                                                                                                                                                                                                                       | `ToolCall` (5.3)                                                                                                                                                                                                                                                                                                                                                  |
| D18 | Attempt logging                    | `turn.rs:1081` (`log_attempt`), `delegate.rs:1332` (`log_attempt`): both call `turn_row` and then insert their own keys into the `Value` (`attempt`, `orchestrated`, `cancelled`, `failover_notes`; `kind`, `planner`, `worker_id`, `task`, `read_only`, `files_changed`, `tool_calls`)                                                                                         | The parts both rows share are typed once: `UsageRow`, `PlanRow`, `error_kind_name` in `backend/infrastructure/outcome_wire.rs`, `SignalsRow` in `judgment`. `TurnLogRow` and `DelegateLogRow` are two structs, each listing all of its keys in alphabetical order (a shared struct with `#[serde(flatten)]` cannot interleave keys, so it would change the bytes) |

### 7.3 How the big files split

These splits and line ranges are as of `296bd5b`. They need a new audit at `ddd-start`.

New files that need a split at `ddd-start` include:

- `privacy.rs`: privacy policy has no exact home in 1.2; use the closest, `approval`.
  Put `Policy`, `Scope` and `Verdict` in `approval/domain/privacy_policy.rs`; path and glob
  matching in `approval/domain/privacy_paths.rs`; `AuditLog` in
  `approval/infrastructure/privacy_audit.rs`.
- `ext/hooks.rs`: extension hooks have no exact home in 1.2; use `approval` for tool-call
  refusal and settings trust. Put trust checks and settings preview in
  `approval/infrastructure/settings_trust.rs`, and hook execution in
  `approval/infrastructure/hooks.rs`. The remaining turn-event hooks need a separate home.
- `sessionlib.rs`: put `Query`, `Hit` and `search` in `session/domain/library_search.rs`;
  scanning and imports in `session/infrastructure/session_library.rs`; `render_hits` in
  `tui/render/session_library.rs`.
- `uifmt/diffview.rs`: put word matching in `tui/render/word_diff.rs`, unified and split
  rows in `tui/render/diff_rows.rs`, and `render_diff` in `tui/render/diff_view.rs`.
- `uifmt/identity.rs`: put colour depth and RGB conversion in `tui/render/color.rs`, identity
  and overrides in `tui/render/identity.rs`, and `ProviderTheme` in
  `tui/render/provider_theme.rs`.
- `uifmt/roles.rs`: put sent, narration and answer blocks in `tui/render/roles.rs`; tool
  grouping and results in `tui/render/tool_rows.rs`; previews in `tui/render/edit_preview.rs`.

The proposed destination files must each fit within the 400-line budget after the split.

**`turn.rs` (2,835: 1,144 production, 1,690 tests)**

| Today                                                                       | Destination                                                                                                                                                      |
| --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Ui` trait, `Answer` 49-101                                                 | `turn/application/ports.rs` (`TurnObserver`)                                                                                                                     |
| `SharedUi`, `EngineApprover` 104-160                                        | `turn/application/approve_tool.rs`                                                                                                                               |
| `Failover`, `NoRouteInfo` 162-187                                           | `turn/domain/failover.rs`, `no_route_info.rs`                                                                                                                    |
| `Cancel` 189-232                                                            | `turn/application/cancel.rs`                                                                                                                                     |
| `TurnStatus`, `TurnReport` 235-271                                          | `turn/domain/turn_report.rs`                                                                                                                                     |
| `Judged` 273-296                                                            | `turn/domain/judged.rs`                                                                                                                                          |
| aliases, `git_status_short`, idle timeout 298-326                           | `ports.rs`, `infrastructure/git_status.rs`, `backend` config                                                                                                     |
| `Chosen`, `Attempt` 329-352                                                 | `turn/domain/attempt.rs`                                                                                                                                         |
| `Engine` struct and builder 354-481                                         | `engine.rs`, `engine_builder.rs`                                                                                                                                 |
| accessors, `new_session`, save, resume 483-609                              | `session_commands.rs`                                                                                                                                            |
| `last_changes`, `last_diff`, `undo_last` 528-555                            | `change_commands.rs`                                                                                                                                             |
| `run*`, `judge` 625-681                                                     | `run_turn.rs`                                                                                                                                                    |
| `run_attempts` 698-809                                                      | `run_attempts.rs` (the loop, with the body of one iteration as its own function)                                                                                 |
| `choose`, `legacy_choice` 810-851, 904-939                                  | `choose_model.rs`                                                                                                                                                |
| `plan_orchestration`, `apply_orchestration`, `mcp_providers` 852-903        | `plan_orchestration.rs`                                                                                                                                          |
| `prompt_for`, `run_attempt` 940-1017                                        | `run_attempt.rs`                                                                                                                                                 |
| `finish`, `cancelled_report`, `record_failed`, `no_route_message` 1018-1139 | `finish_turn.rs`                                                                                                                                                 |
| `log_attempt` 1081-1104                                                     | `infrastructure/turn_log.rs`                                                                                                                                     |
| tests 1145-2835                                                             | `turn/application/tests/`: `fakes.rs`, `routing.rs`, `failover.rs`, `handoff.rs`, `approval.rs`, `orchestration.rs`, `cancel.rs`, `changes.rs`, `persistence.rs` |

**`delegate.rs` (2,856: 1,643 production)**

| Today                                                                                | Destination                                                                            |
| ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| `BriefSignals`, `worker_task`, `worker_floor` 85-131                                 | `orchestration/domain/brief_signals.rs`                                                |
| `NoWorker`, `WorkerView`, `candidate_lists`, `spread`, `pick_worker` 132-216         | `domain/worker_pick.rs`                                                                |
| patch parsing and paging 218-347                                                     | `domain/file_patch.rs`, `patch_page.rs`                                                |
| `worker_prompt` 349-365                                                              | `domain/worker_prompt.rs`                                                              |
| `Worktrees` trait 367-385                                                            | `application/ports.rs`                                                                 |
| `git`, `Snapshot`, `explain_apply_failure`, `checked_inside`, `GitWorktrees` 386-573 | `infrastructure/git_worktrees.rs`, `worktree_base.rs` (the second `Snapshot`, renamed) |
| `Worker`, `Owner` 575-640                                                            | `domain/worker.rs`                                                                     |
| `Liveness`, `ProcLiveness` 641-675                                                   | `ports.rs`, `infrastructure/process_liveness.rs`                                       |
| `Saved`, `Records`, `Store` 676-790                                                  | `infrastructure/worker_file.rs`, `worker_wire.rs`; trait to `ports.rs`                 |
| `Found`, `Fate` 796-840                                                              | `domain/recovery.rs`                                                                   |
| `has_leftovers` 791-794, `Scan` 841-878 (they read directories, V17)                 | `infrastructure/worker_file.rs`                                                        |
| `DelegateRequest`, `DelegateResult` 880-962                                          | `domain/delegation.rs`, `delegation_result.rs`                                         |
| `DelegateApi`, `Judge` 963-988                                                       | `ports.rs`                                                                             |
| `CreateSlot`, `Reservation` 1000-1083                                                | `application/reservation.rs`                                                           |
| `Delegator` struct and builder 1084-1277                                             | `application/delegator.rs`                                                             |
| `recover` 1278-1318                                                                  | `application/recover_workers.rs`                                                       |
| `run_worker`, `attempt`, `run_attempt` 1319-1442                                     | `application/run_worker.rs`                                                            |
| `delegate` 1443-1476                                                                 | `application/delegate_work.rs`                                                         |
| `show_diff` 1477-1539                                                                | `application/show_worker_diff.rs`                                                      |
| `merge`, `discard` 1540-1603                                                         | `application/merge_worker.rs`, `discard_worker.rs`                                     |

**`backend/mod.rs` (1,874: 964 production)**

| Today                                                                            | Destination                                                                                                                                          |
| -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ProviderId` 31-76                                                               | `shared/domain/provider_id.rs`                                                                                                                       |
| `Usage` 77-85                                                                    | `shared/domain/token_usage.rs`                                                                                                                       |
| `ErrorKind`, `PlanUsage`, `Finish`, `Event` 86-139                               | `backend/domain/error_kind.rs`, `plan_usage.rs`, `event.rs`                                                                                          |
| `PermissionRequest`, `Decision`, `Approver`, `NoApprover` 142-199                | `approval/domain/permission_request.rs`, `decision.rs`; `Approver` to `backend/application/ports.rs`                                                 |
| `TurnRequest`, `TurnOutcome`, `TurnError` 201-240                                | `backend/domain/`                                                                                                                                    |
| `McpAttachment` 242-251                                                          | `backend/domain/mcp_attachment.rs`                                                                                                                   |
| `Backend` trait 253-300                                                          | `backend/infrastructure/cli_dialect.rs` (it returns a `tokio::process::Command`, so it is an adapter contract); the application port is `TurnRunner` |
| text classification 303-387                                                      | `backend/domain/error_text.rs`                                                                                                                       |
| `PromptArg`, prompt files 389-507                                                | `domain/prompt_arg.rs`, `infrastructure/process/prompt_files.rs`                                                                                     |
| `StdinFeed`, stderr capture, `Pump` 509-775                                      | `process/stdin_feed.rs`, `stderr_tail.rs`, `output_pump.rs`, `idle_watchdog.rs`                                                                      |
| `judge`, `reap`, `explain_eof`, `run_turn`, `run_turn_with`, `run_child` 776-942 | `process/child.rs`, `cli_turn_runner.rs`                                                                                                             |
| `GroupKill`, `process_alive`, `is_zombie` 843-963                                | `process/process_group.rs` over `sys_unix`                                                                                                           |

**`changes.rs` (1,586: 820 production)**: types 31-117 to `domain/file_change.rs`,
`snapshot.rs`, `change_set.rs`; git output parsing 119-183 and 660-670 to
`infrastructure/git_output.rs`; `merge_untracked` 185-243 to `domain/untracked_merge.rs`;
temp index 266-325 to `infrastructure/git_temp_index.rs`; `snapshot`, `summarize`, `diff_text`
327-422 to `infrastructure/git_working_tree.rs` behind the `WorkingTree` port; the pure half
of the new-file diff (`looks_binary`, `count_lines`, `render_new_file`, 424-458) to
`domain/new_file_diff.rs`, and the half that reads the file (460-497) to
`git_working_tree.rs`; the cancellable wrappers 502-520 to `application/track_turn.rs`; undo
planning 577-658 to `domain/undo_plan.rs`, `safe_path.rs`; the path guards that look at the
disk (672-713) to `infrastructure/undo_files.rs`; the 67-line `undo` 714-783 to
`application/undo_turn.rs` in three named steps (plan, verify nothing changed since, apply);
file restoring 785-820 to `infrastructure/undo_files.rs`.

**`permissions.rs` (1,348: 840 production)**: review text 30-240 to `approval/domain/review.rs`
and `visible_text.rs`; rule policy 244-500 to `rule_policy.rs`, `bash_rule.rs`,
`permission_rule.rs`; `Always` 508-560 to `always_plan.rs`; `validate_rule` 562-586 to
`permission_rule.rs`; the settings document 588-760 to
`infrastructure/claude_settings_file.rs` and `claude_settings_wire.rs`; `resolve_dir` 763-778
to `infrastructure/directory_path.rs`; `Listing` 780-816 to `domain/listing.rs` and
`application/list_rules.rs`; `/permissions` argument parsing 818-840 to
`tui/permissions_command.rs` (it is command-line syntax, not approval policy).

**`console.rs` (1,433: 676 production)**: `Console`, `ConsoleConfig`, `Inner` output
serialisation to `tui/console/console.rs`; `Ticker` and the live status line to
`status_ticker.rs`; `impl Ui for Console` (457-553) to `turn_observer.rs`; the approval
question, key reading (`TerminalKey` 67-107) and "always" handling (587-666) to `approval_prompt.rs`; `Buf` test
writer to `testkit`.

**`router.rs` (1,197: 832 production)**: `Sticky` (157-200) to
`routing/domain/legacy_tier_route.rs`; `Decision` (14-19, becomes `TierRoute`) and `CodeWant` to
`judgment/domain/tier_want.rs`, because `Router::route` returns `Decision` and `judgment` may
not import `routing`; `ProviderWant` to `judgment/domain/judgment.rs`; thresholds 57-151 to `thresholds.rs`; `Orchestration` 88-136 to
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

1. One step is one or more commits; after **every commit** `scripts/check.sh` passes. A commit
   either moves code or changes code, never both.
2. Files move with `git mv`, so `git log --follow` keeps working (tried: it follows a file out
   of `src/bin/` even when a new stub takes the old path in the same commit). A split comes in
   later commits: the original file keeps the largest part (it carries the history), the
   extracted parts are new files (`git blame -C -C` still finds their origin).
3. A move commit is proved to be a move by `scripts/move_check.py HEAD`: for every renamed
   file, the removed and added lines are equal as multisets, apart from lines matching an
   allow list (`mod`, `use`, `pub use`, visibility keywords, `#[cfg(...)]` on a test seam,
   `Cargo.toml` lines). A `git diff --stat` cannot show this; the script can.
4. Old paths keep working from the extraction (P1b) until P5-1 through each crate's
   `legacy.rs`: one module per old flat module, under its old name
   (`pub mod router { pub use crate::application::judge::*; }`), re-exported from `lib.rs`
   (`pub use crate::legacy::{router, typesafe};`). They are listed in the baseline and removed
   in P5-1. This lets lanes split different crates at the same time.
5. **Ownership is by crate.** In a parallel phase one worker owns one `crates/<context>/`
   folder, its `Cargo.toml` and its baseline file, and edits nothing else. The root
   `Cargo.toml`, `Cargo.lock`, `clippy.toml`, `scripts/`, `tests/architecture.rs`,
   `crates/shared/` and `crates/testkit/` are edited only in serial steps.
6. **Expand, then contract.** A lane never changes or removes a `pub` item that another crate
   uses, because that would break a crate the lane does not own. It adds the new item beside
   the old one. A serial step after the lanes switches the callers and deletes the old item.
   A change to a type that crosses crates (`Event`, `Signals`, `ModelId`) is always serial.
7. No reformatting. The code is not `rustfmt`-default formatted today (138 lines of `turn.rs`
   alone exceed 100 columns), and a reformat inside a move would destroy rename detection.
   One `rustfmt` commit after `ddd-done` is recommended (U5), with a committed `rustfmt.toml`
   and the commit listed in `.git-blame-ignore-revs`.
8. Nothing observable changes: binary names, flags, every environment variable, file names
   and formats under the state directory, log row bytes, CLI argument lists, exit codes, screen
   text. The golden tests, `scripts/test_pty_ux.py` and `scripts/parity.py` check this.
9. Freeze: all open work (`fix-orch-trigger`, review fixes, the worker now fixing code on its
   own branch) merges first. The merged commit is tagged `ddd-start`, and P0-0 pins this
   document to it (it refers to `296bd5b`, and code has been merged since). Feature work is
   frozen from P1a to `ddd-p1`.
10. No step runs `grok`, `agy`, `gemini`, `claude` or `codex`, and none calls Jev. All tests
    use the existing fakes; CI has none of the CLIs installed (3.6).
11. No secret enters the repository, a golden, a fixture or a log of the migration. Goldens
    are built from literal fake values (`"k"`, `"sk-test"`), never from the developer's
    environment; `check.sh` clears the key variables and hides the env file before running
    (3.5).

### 8.2 Steps

Risk: L = mechanical, compiler-checked; M = structural, covered by existing tests; H = changes
how data is parsed or written, needs the oracle or golden tests named in the step.

**Phase 0: safety net (serial, in today's single crate)**

| Step | What                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Risk | Verification beyond the gate                                                                                                         |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------ |
| P0-0 | Tag `ddd-start`. Pin this document to it: every `file:line`, the figures of the header, 3.2, 5.4-5.6, 7.1-7.3 and 9.2, and the import graph behind 1.3 (derived from the `use` items, as for this version). The splits of 7.3 are rewritten for the files changed since `296bd5b`.                                                                                                                                                                                                                                                                                                          | L    | A reviewer checks 30 citations chosen at random; the derived graph has no edge outside the table of 1.3 other than those P1a removes |
| P0-1 | `scripts/check.sh` (3.5), `rust-toolchain.toml`, `.github/workflows/check.yml` (3.6). Fix the default clippy findings (23 as of `296bd5b`) and move the code that follows the test module in `anthropic.rs`, one lint per commit.                                                                                                                                                                                                                                                                                                                                                                         | L    | The script passes locally and in CI                                                                                                  |
| P0-2 | Add `fixture!` (in a new test-only module `src/testkit/fixture.rs`) and replace the 24 relative fixture `include_str!` paths (6 in `backend/claude.rs`, 3 in `codex.rs`, 10 in `gemini.rs`, 4 in `grok.rs`, one in `permissions.rs`). Also preserve the relative `docs/CHANGELOG.md` include in `learn.rs`.                                                                                                                                                                                                                                                                                                                                                                 | L    | Same tests pass                                                                                                                      |
| P0-3 | Goldens (4.1), written from today's code: turn log row, delegate log row, `jev-ask` line, gateway request/turn/decision/check records, `cooldowns.json`, saved session file, worker record, MCP config file, `settings.local.json` after each kind of edit, the Jev request body, Claude `control_response` lines, the forwarded gateway body on both `set_model` paths, the usage log's `prefix` field (5.2), and each backend's argument list. Two cases each (5.3).                                                                                                                      | L    | Each golden is read by a reviewer; each test fails when one byte of its golden is changed                                            |
| P0-4 | `tests/architecture.rs` with its lexer tests, check T1 (flat modules against the map) and the baseline.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | L    | Passes with the baseline; removing any baseline line fails; a planted violation fails                                                |
| P0-5 | `scripts/budget.py` in report mode, `scripts/move_check.py`, `scripts/parity.py`: checks out `ddd-start` into a temporary directory outside the repository, builds it and HEAD each in its own `target/`, and compares, in a hermetic `HOME` with fake CLIs on `PATH`, the stdout, stderr and exit code of `jev-code --help`, `jev-code doctor`, `jev-code -p` against the fake `claude`, `--version` of each binary that has it, `jev-delegate-mcp` `initialize` and `tools/list`, `jev-ask` against a fake CLI, and the files each run leaves in the state directory (timestamps masked). | L    | Parity of the tree against itself is empty; a planted text change is reported                                                        |
| P0-6 | Gather the duplicated test scaffolding of D16: context-free helpers into `src/testkit/`, the scripted fake backend into `src/backend/testing.rs`. One helper kind per commit (4 commits).                                                                                                                                                                                                                                                                                                                                                                                                   | M    | Test count unchanged (769 as of `296bd5b`)                                                                                                           |
| P0-7 | Tag `ddd-base`: the tree with the safety net in place. Goldens, fixtures and scripts are compared with this tag from here on; behaviour is compared with `ddd-start`, so the code edits of P0-1 and P0-6 are covered too.                                                                                                                                                                                                                                                                                                                                                                   | L    | `parity.py ddd-start` prints no difference                                                                                           |

**Phase 1a: untangle in place (serial, still one crate)**

Crates cannot have cycles, and today's modules do (1.3). Each step moves a few items to the
module of the context that owns them, always upstream, so the old path can stay as a
re-export for callers whose context may import the old module. Callers whose context may not
(`catalog`, `routing`, `doctor` and `bin/route.rs` name `backend::ProviderId` today) are
rewritten to the new path in the same step. After the last step the T1 baseline is empty: the
flat modules already form the map of 1.3, and every later extraction is a file move.

| Step   | Items                                                                                                                                                       | From                        | To                                        | Edge removed                                             |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- | ----------------------------------------- | -------------------------------------------------------- |
| P1a-1  | `ProviderId`, `Usage`                                                                                                                                       | `backend/mod.rs:31-85`      | new `src/shared/` (with `envfile`, `log`) | `catalog`, `routing`, `handoff`, `cooldown` -> `backend` |
| P1a-2  | `MODEL_CHEAP/MID/STRONG/TOP`                                                                                                                                | `anthropic.rs`              | `catalog`                                 | `router`, `routing`, `turn` -> `anthropic`               |
| P1a-3  | `is_limit_phrase`, `is_limit_message`                                                                                                                       | `anthropic.rs`              | `shared` (`limit_notice.rs`; done at `ddd-base`+: `anthropic` calls them too and may not import `backend`) | `backend` -> `anthropic`                                 |
| P1a-4  | `PermissionRequest`, `Decision`                                                                                                                             | `backend/mod.rs:142-199`    | `permissions`                             | the approval types sit below their user                  |
| P1a-5  | `backend_for`, `session_backend_for`, `deepseek_env`, gemini flavor (`providers.rs:43-116`); `installed_for_real`, PATH and mise checks (`availability.rs`) | `providers`, `availability` | `backend`                                 | prepares `providers -> backend` only                     |
| P1a-6  | `install_hint`, `login_command`, `login_instruction`                                                                                                        | `doctor.rs:218-259`         | `providers`                               | `uifmt -> doctor`                                        |
| P1a-7  | `write_mcp_config`                                                                                                                                          | `orchestrate.rs:172`        | `backend/claude.rs`                       | `orchestrate` -> a `backend` adapter                     |
| P1a-8  | `BackendFactory`, `idle_timeout_from_env`, `idle_timeout`                                                                                                   | `turn.rs:298-326`           | `backend`                                 | `delegate -> turn`                                       |
| P1a-9  | all of `turnlog.rs`                                                                                                                                         | `turnlog`                   | `backend`                                 | `delegate -> turnlog`                                    |
| P1a-10 | `Sticky`                                                                                                                                                    | `router.rs:157-200`         | `routing`                                 | a hold rule leaves `judgment`                            |
| P1a-11 | `RouteFacts` (a plain struct) to `turn`; `denied_without_asking` (approval wording, `turn.rs:142`) to `permissions`                                         | `uifmt`                     | `turn`, `permissions`                     | `turn -> uifmt`                                          |
| P1a-12 | the test at `doctor.rs:943`                                                                                                                                 | `doctor`                    | `uifmt/failure.rs` tests                  | `doctor -> uifmt` (test only)                            |
| P1a-13 | Project settings trust decision and the configuration path it needs | `ext/hooks.rs`, `ext/mod.rs`, `backend/claude.rs` | `approval` and a trust fact on the backend request | `backend -> ext` for settings trust |

Tag `ddd-p1a`. Verification for every step: `move_check.py`, and the T1 baseline shrinks.

**Phase 1b: extract the crates, leaves first (serial, scripted)**

| Step             | What                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Risk                                                                                                                                               |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1b-0            | Make the workspace. `git mv src crates/cli/src`, `git mv Cargo.toml crates/cli/Cargo.toml`; new virtual root `Cargo.toml` with the three workspace lines of 4.3, `[workspace.package]` and `[workspace.dependencies]` copied from today's versions and features; `fixture!` and `golden!` gain `/../..`; `crates/cli/Cargo.toml` registers `[[test]] architecture` with `path = "../../tests/architecture.rs"`, and the test's source root becomes `crates/`. `Cargo.lock` stays at the root. Package `jev-router` now lives in `crates/cli` and still holds everything. If Flashboard already exists, see 4.4. | M: `git diff ddd-p1a -- Cargo.lock` shows no package added or changed; `scripts/install.sh "$tmp"` works; `cargo build` prints no resolver warning |
| P1b-1 ... P1b-17 | One crate per step, in the order below.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | L                                                                                                                                                  |
| P1b-18           | The eight binaries: each body moves with `git mv` into its context crate (`crates/cli/src/bin/gateway.rs` to `crates/gateway/src/interface/http/proxy_handler.rs`, with its tests) and a new root of 30 lines or fewer takes its place. `crates/cli/src/lib.rs` is deleted; binaries name `jev_<context>::`.                                                                                                                                                                                                                                                                                                    | M: `parity.py` empty                                                                                                                               |
| P1b-19           | `crates/embed` with the six contract names of 4.4 and `tests/embed_api.rs` (names and signatures). `parse_codex_login` becomes `pub`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | L: with Flashboard present, its `engine_bridge` contract tests pass                                                                                |

Order of P1b-1 to P1b-17 (the levels of 1.3): `testkit`, `shared`, `catalog`, `approval`,
`changes`, `judgment`, `backend`, `routing`, `providers`, `session`, `orchestration`, `doctor`,
`chat`, `turn`, `gateway`, `tui`, `calibration` (created empty here, filled in P1b-18).
`embed` is created in P1b-19 and completed in P5-4.

What one extraction step does, the same every time:

| #   | Action                                                                                                                                                                                                                                                                                                                                                                                               | Effect                                                                                                                                                                                                                                                        |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Create `crates/<context>/Cargo.toml` and `README.md` (purpose line only).                                                                                                                                                                                                                                                                                                                            |                                                                                                                                                                                                                                                               |
| 2   | `git mv` the context's files to the first destination named in 4.5.                                                                                                                                                                                                                                                                                                                                  | The file changes only by the edits of row 5, so rename detection keeps it                                                                                                                                                                                     |
| 3   | Write `src/lib.rs` and `src/legacy.rs`: the `mod` lines; one `legacy` module per old flat module, under its old name, re-exporting the moved file by glob (`pub mod availability { pub use crate::domain::availability::*; }`); `pub use crate::legacy::{availability, cooldown, providers};`; and private aliases for the upstream paths the files already use (`use jev_shared::{envfile, log};`). | `crate::envfile::lookup` and `crate::cooldown::Cooldowns` inside a moved file still resolve, so the file needs no edit. One module per old name, because a context holds up to three old modules and a single glob of all of them could make a name ambiguous |
| 4   | In `crates/cli/src/lib.rs`, replace `pub mod availability; pub mod cooldown; pub mod providers;` by `pub use jev_providers::{availability, cooldown, providers};`.                                                                                                                                                                                                                                   | Every module not yet extracted, and every binary, compiles unchanged                                                                                                                                                                                          |
| 5   | Fix what the compiler reports, and only that: a `use super::...` line whose parent moved (the four CLI files of `backend/`, the files of `uifmt/`; every `super::` in production code is on a `use` line today); `pub(crate)` items another crate uses become `pub`; a `#[cfg(test)]` seam used from another crate (`Runtime::for_tests`) becomes `#[cfg(any(test, feature = "testing"))] pub`.      | The only edits inside moved files; `move_check.py` allows exactly these                                                                                                                                                                                       |
| 6   | `ARCH_BASELINE=write`, gate.                                                                                                                                                                                                                                                                                                                                                                         | The moved file's layer violations enter that crate's baseline                                                                                                                                                                                                 |

Tag `ddd-p1`. Measure clean build, test build and one-file rebuild again and write them in 3.2.

**Phase 2: split inside each crate (parallel, one lane per crate)**

Each lane is a sequence of extraction commits following 7.3 and 4.5, one destination file per
commit: move the items to the new file in the right layer folder, keep them reachable under
the old path through `legacy.rs`, move their tests with them. No signature changes. A lane
touches only its crate folder and its own baseline file.

| Lane | Crate                                | Commits (about) | Risk                                                                                             |
| ---- | ------------------------------------ | --------------: | ------------------------------------------------------------------------------------------------ |
| P2-a | `shared` (serial, before the others) |               6 | L                                                                                                |
| P2-b | `catalog`                            |               8 | L                                                                                                |
| P2-c | `judgment`                           |              10 | M                                                                                                |
| P2-d | `routing`                            |               8 | L                                                                                                |
| P2-e | `approval`                           |              10 | M                                                                                                |
| P2-f | `backend`                            |              22 | M (the process runner is timing-sensitive; its tests at `backend/mod.rs:965-1874` run unchanged) |
| P2-g | `providers`                          |              20 | M                                                                                                |
| P2-h | `session`                            |               9 | L                                                                                                |
| P2-i | `changes`                            |              12 | M                                                                                                |
| P2-j | `orchestration`                      |              30 | M                                                                                                |
| P2-k | `turn`                               |              20 | M                                                                                                |
| P2-l | `doctor`, `chat`, `calibration`      |              14 | L                                                                                                |
| P2-m | `gateway`                            |              22 | M                                                                                                |
| P2-n | `tui`                                |              25 | M (PTY test is the judge)                                                                        |

Tag `ddd-p2`. Every file is in its final folder and within the file budget; names are
unchanged.

**Phase 3: seams and duplication**

| Step | What                                                                                                                                                                                                                                                                                                                                                                     | Mode                                                         | Risk                                                                                             |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| P3-1 | `shared`: `Env` port, `EnvVar` enum, `Secret`; `Clock`, `StateDir` ports; `private_file`, `git_command`, `lock`, `truncation`, `duration_text`, `unix_millis`. The old free functions stay and delegate to the new code.                                                                                                                                                 | serial                                                       | M                                                                                                |
| P3-2 | Each crate adds `config.rs`, takes configuration and clock through **new** constructors beside the old ones, and replaces its copies of D1-D15 with the shared ones.                                                                                                                                                                                                     | parallel, one lane per crate                                 | M (D4: keep each variable's exact parsing; a test per variable)                                  |
| P3-3 | Ports: `TurnRunner`, `BackendFactory`, `McpConfigWriter` in `backend`; `WorkingTree` in `changes`; `SessionStore` in `session`; `CooldownStore` in `providers`; `JudgmentService` in `judgment`; `Worktrees`/`Records`/`Liveness` to `ports.rs`. Adapters implement them beside the closure aliases.                                                                     | parallel                                                     | M                                                                                                |
| P3-4 | Contract: `tui`, `cli` and the `compose.rs` files switch to the new constructors and ports; the old constructors, the closure aliases and the free functions `envfile::lookup` and `log::now_ms` are deleted. `state_dir`, `env_file_path` and `parse` stay as they are: the facade exports them (4.4) and `embed_api.rs` pins their signatures. A8 leaves the baseline. | serial                                                       | M                                                                                                |
| P3-5 | `turn`: `Ui` becomes `TurnObserver` + `Approver` with typed facts; `tui/console/turn_observer.rs` produces the same text. `ProviderRegistry` hides its fields. `render*` leaves `providers` for `tui`.                                                                                                                                                                   | serial (touches `turn`, `tui`, `orchestration`, `providers`) | H (screen text: PTY test plus `parity.py`)                                                       |
| P3-6 | Layer cleanup V5-V7 and V17 (rules that read the disk get the fact as an argument): A1, A4, A5 leave the baseline.                                                                                                                                                                                                                                                       | parallel                                                     | M (V17 touches the "always" rules, a security boundary: every existing rule test runs unchanged) |

Tag `ddd-p3`.

**Phase 4: type strictness**

| Step  | What                                                                                                                                                                                                                                                                                                                                                                                           | Mode                                                 | Risk                                                                                         |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| P4-1  | `shared`: `ModelId`, `Probability`, `UnixMillis`, `TokenUsage`, `OpaqueJson`; `jsonl_log::append<T: Serialize>` beside the old one.                                                                                                                                                                                                                                                            | serial                                               | L                                                                                            |
| P4-2  | `backend`: typed wire enums for claude, codex, gemini, agy, grok, each with the differential oracle of 5.3 (one commit adds the typed parser and oracle, the next deletes the old parser). Private to the crate.                                                                                                                                                                               | lane                                                 | H                                                                                            |
| P4-3  | `judgment`: typed `Question`, `Answers` inside the crate; golden Jev request body unchanged.                                                                                                                                                                                                                                                                                                   | lane                                                 | H                                                                                            |
| P4-4  | `session`, `providers`, `catalog`, `orchestration` (worker record): file DTOs. A file written by the current release must still load: a test loads a file captured at `ddd-base`.                                                                                                                                                                                                              | 4 lanes                                              | H                                                                                            |
| P4-5  | `orchestration/interface/mcp`: typed JSON-RPC.                                                                                                                                                                                                                                                                                                                                                 | lane                                                 | M                                                                                            |
| P4-6  | `gateway`: `MessagesRequest` view, `UsageRecord`, `SseEvent`; `Value` confined to `anthropic_wire.rs`. Golden upstream body on both `set_model` paths.                                                                                                                                                                                                                                         | lane                                                 | H                                                                                            |
| P4-7  | Cross-crate types, one per commit, each expand-then-contract: `Signals` replaces `signals: Value` (`judgment`, then `turn`, `orchestration`, `gateway`, `backend`'s `outcome_wire`); `ToolCall` replaces `Event::Tool(String)` (`backend`, then `session`, `turn`, `tui`); `ToolInput` replaces `PermissionRequest.input` (`approval`, then `backend`, `tui`); `TurnLogRow`, `DelegateLogRow`. | serial                                               | H (golden rows; rule policy is a security boundary, every existing rule test runs unchanged) |
| P4-8  | `approval`: settings wire file; `permission_wire.rs` with `OpaqueJson`.                                                                                                                                                                                                                                                                                                                        | lane (after P4-7)                                    | H                                                                                            |
| P4-9  | Typed errors: one `Error` per crate; `Result<_, String>` and `anyhow` removed from the libraries. User-visible error text is pinned by tests first. Callers in other crates switch in a serial commit per crate.                                                                                                                                                                               | lanes, then serial                                   | M                                                                                            |
| P4-10 | Newtypes and enums of 5.4 (`LegacyTier`, `Role`, `PlanWindow`, `WorkerId`, `PermissionRule`, `ModelId` adoption).                                                                                                                                                                                                                                                                              | serial where the type crosses crates, lane otherwise | M                                                                                            |
| P4-11 | Remove the 11 panic sites and the `as` casts; move `unsafe` to `sys_unix.rs` (then `libc` leaves every `Cargo.toml` but `shared`'s); replace the downcast.                                                                                                                                                                                                                                     | lanes, then serial for `sys_unix`                    | M                                                                                            |
| P4-12 | Turn on the lint tables of 5.6 one group at a time (`unsafe_code` and `forbid`, the `unwrap_used` group, `indexing_slicing` and `string_slice`, `as_conversions`, `disallowed_*`, `print_*`, then `pedantic`), one commit per group and crate. A6, A7 leave the baseline.                                                                                                                      | serial per group, lanes per crate inside a group     | M (866 findings today, 5.6)                                                                  |

Tag `ddd-p4`.

**Phase 5: closing (serial)**

| Step | What                                                                                                                                                                                                                                                                                                                                 | Risk |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---- |
| P5-1 | Delete every `legacy.rs` and the lib-root aliases; rewrite remaining old paths to `jev_<context>::Name` (scripted); `lib.rs` re-exports become explicit lists; layer modules become private.                                                                                                                                         | L    |
| P5-2 | Rename to the glossary: `Runtime` to `ProviderRegistry`, `ChangeSummary` to `ChangeSet`, `ProviderWant` to `Judgment`, `Sessions` to `Session`, the second `Gate`/`Snapshot`/`Decision`/`SessionInfo`/`Interrupt`/`Usage`, and the second `run_turn` (1.1). One rename per commit. The six names the facade exports are not renamed. | L    |
| P5-3 | Function budget: split the functions still over 60 lines.                                                                                                                                                                                                                                                                            | M    |
| P5-4 | `crates/embed` gains the chat-window names of 4.4, each pinned in `embed_api.rs`; its manifest takes its full row of 1.3.                                                                                                                                                                                                            | L    |
| P5-5 | Write every crate `README.md` from the template (9.3); `//!` owns/must-not on every module; A10-A14, C1-C3 leave the baseline.                                                                                                                                                                                                       | L    |
| P5-6 | `budget.py` to failing mode; delete `tests/architecture_baseline/` (every file is empty); update the "Files" section of `README.md` (lines 476-500), its line 334 and `scripts/provider_eval_README.md:23` (`grep -n 'src/' README.md scripts/*.md` then shows only example prompts); add `scripts/dod.sh` (section 10).             | L    |
| P5-7 | Run `scripts/dod.sh`; tag `ddd-done`.                                                                                                                                                                                                                                                                                                | L    |

**Count.** 8 + 12 + 20 + 14 lanes + 6 + 12 + 7 steps; about 430 commits: P0 16, P1a 12,
P1b 21, P2 216, P3 60, P4 90, P5 20.

**Is that realistic, and is there a faster path that is as safe?** The gate costs about two
minutes (3.5), so 430 commits are about 15 hours of gate time; with the P2 lanes in parallel
the wall time is set by the longest lane (30 commits, one hour of gate). What must not be done
to go faster: merging moves with changes, or batching several destination files into one
commit; both make a failure hard to locate and a revert wide. What is done:

| Saving                                   | How                                                                                                                                                        | Why it is as safe                                                                                                                                          |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Lanes run a fast gate per commit         | `scripts/check.sh --fast`: clippy and tests for the lane's crate and the crates that depend on it (`cargo test -p`), architecture test, budget. About 40 s | The integrator replays every lane with `git rebase --exec scripts/check.sh`, so no commit reaches `refactor/ddd` without the full gate, PTY suite included |
| P1a and P1b are scripted                 | One script per phase, reviewed once, applied step by step                                                                                                  | Each step is still its own commit and passes the gate and `move_check.py`                                                                                  |
| No separate "move to layer folder" phase | P1b puts each file directly at its final first destination                                                                                                 | One rename per file instead of two                                                                                                                         |

### 8.3 Parallel work without conflicts

| Phase            |                   Workers | Each worker owns                                                        | Shared files, and who edits them                                      |
| ---------------- | ------------------------: | ----------------------------------------------------------------------- | --------------------------------------------------------------------- |
| P0, P1a, P1b     |                         1 | everything                                                              | n/a                                                                   |
| P2               | up to 13 (after `shared`) | one `crates/<context>/` and `tests/architecture_baseline/<context>.txt` | none                                                                  |
| P3-2, P3-3, P3-6 |                  up to 14 | the same                                                                | none: rule 6, callers switch in P3-4                                  |
| P4 lanes         |                   up to 8 | the same, plus `tests/golden/<context>/` read-only                      | root `Cargo.toml` (a new feature of an outside crate): the integrator |
| P5               |                         1 | everything                                                              | n/a                                                                   |

Why lanes cannot collide: a lane's files are all under one crate folder; the root manifest
lists members by glob (`crates/*`), so no lane edits it; `Cargo.lock` does not change because
no lane may add an outside dependency; baselines and goldens are per context. Each worker uses
its own git worktree and branch (`ddd/<phase>-<context>`). A worktree has its own `target/`,
and `CARGO_TARGET_DIR` stays unset: the PTY suite runs `<worktree>/target/debug/code`, so a
shared or redirected target folder would test another lane's binary. Worktrees inside
`.claude/worktrees/` are their own workspaces (each has the root manifest) and the `exclude`
line keeps the main checkout from claiming them. The integrator rebases lanes onto
`refactor/ddd` one at a time with the full gate on every commit.

### 8.4 Verification

| After             | Run                                                                                                                                                                                                                                                            |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| every commit      | `scripts/check.sh` (3.5): clippy with warnings denied, Rust tests including the architecture and golden tests, PTY UX test, three Python test files; the same in CI                                                                                       |
| every move commit | `scripts/move_check.py HEAD`                                                                                                                                                                                                                                   |
| every phase       | `scripts/parity.py ddd-start`: empty diff; the sum of passed tests over all crates is not lower than before; `python3 -I scripts/budget.py` offender count not higher than before; `git diff ddd-base -- tests/golden tests/fixtures scripts/install.sh` empty |
| P4 lanes          | the differential oracle and golden tests named in the step                                                                                                                                                                                                     |
| the end           | `scripts/dod.sh` (section 10)                                                                                                                                                                                                                                  |

Review loop: after each phase a reviewer who did not write it reads the phase diff; findings
are fixed and the review repeats until a round finds nothing, as for all work on this project.

### 8.5 Rollback

- All work is on `refactor/ddd`; `main` and the installed binaries are untouched until the
  final merge.
- Every commit is green, so any single step is undone with `git revert <sha>`: a move commit
  reverts to the old location, an extraction reverts to the unsplit file. A crate extraction
  (P1b) is reverted in reverse order only, since later crates depend on earlier ones.
- Phase tags (`ddd-base`, `ddd-p1a`, `ddd-p1` ... `ddd-p4`, `ddd-done`) are safe stopping
  points: each is a working, shippable tree. If the work must stop, the branch is merged at
  the last tag; the ratchet baseline documents exactly what remains.
- A P4 step that turns out to change bytes (a golden or oracle test cannot be made to pass
  without changing behaviour) is not forced: the step is dropped and the user is asked whether
  to add a fourth allow-listed wire file (U3 covers three).
- After the final merge, a regression in the field is handled by reinstalling from the
  `ddd-base` tag (`git checkout ddd-base && scripts/install.sh`); state files and logs are
  format-compatible in both directions because no format changed.

---

## 9. Readability rules

### 9.1 The one-minute test

A person who has read only the root `README.md` section "Layout" (16 lines, one per context)
and one context `README.md` can name the file that holds a given behaviour. The definition of
done includes ten such questions with their expected answers, for example:

| Question                                                        | Answer                                       |
| --------------------------------------------------------------- | -------------------------------------------- |
| Where is it decided that a correction keeps the stronger model? | `routing/domain/switch_gate.rs`              |
| Where is the wording of the Jev task-type question?             | `judgment/domain/question_catalog.rs`        |
| Where does a Codex output line become an Event?                 | `backend/infrastructure/codex/codex_wire.rs` |
| Where is "Bash(cargo test:\*)" derived from a command?          | `approval/domain/bash_rule.rs`               |
| Where is the length of a cooldown computed?                     | `providers/domain/cooldown_length.rs`        |
| Where is the catch-up shortened to fit its budget?              | `session/domain/catch_up.rs`                 |
| Where does `/undo` refuse because a file changed since?         | `changes/domain/undo_plan.rs`                |
| Where is a worker model picked?                                 | `orchestration/domain/worker_pick.rs`        |
| Where does the gateway swap `jev-code` for a real model?        | `gateway/application/choose_model.rs`        |
| Where is the terminal put back after a panic?                   | `tui/tty/panic_hook.rs`                      |

### 9.2 Budgets, enforced by `scripts/budget.py`

| Budget                                                                                                                   | Limit | Today                                                             |
| ------------------------------------------------------------------------------------------------------------------------ | ----: | ----------------------------------------------------------------- |
| Lines per `.rs` file, including inline tests                                                                             |   400 | 29 files exceed it on production code alone; the largest file is 3,911 lines |
| Lines per test-only file (`*_tests.rs`, `tests/*.rs`)                                                                    |   600 | n/a                                                               |
| Lines per function (also `clippy::too_many_lines`)                                                                       |    60 | 12 production functions exceed it; the largest is 145 (as of `296bd5b`)             |
| Function parameters                                                                                                      |     6 | 2 `#[allow(clippy::too_many_arguments)]`                          |
| `pub use` names in a crate's `lib.rs`, grouped under the four headings of the README (types, use cases, ports, adapters) |    40 | n/a                                                               |
| Files in one folder (`crates/` holds 19 crates)                                                                          |    20 | `src/` has 85 `.rs` files side by side                                |
| `crates/cli/src/bin/*.rs` lines                                                                                          |    30 | up to 1,265                                                       |
| Lines in a member `Cargo.toml` (the eight `[[bin]]` entries of `cli` aside)                                              |    25 | n/a                                                               |

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
6. The public surface of a context is the `pub use` list of its `lib.rs`, repeated in its
   README. Nothing else is importable.
7. Every context README follows one template:

```
# <context>
<One sentence: the purpose.>

## Language        the glossary terms this context owns, one line each
## Public surface  the names exported by lib.rs, grouped: types, use cases, ports, adapters
## Layout          one line per file: what it owns
## Depends on      the contexts it may import, and what it takes from each
## Must not        the things this context never does (for example: "never reads the clock")
## Where is ...    five questions a newcomer asks, each answered with a file name
```

### 9.4 Before and after: the turn context

**Before.** One file, `src/turn.rs`, 2,835 lines. To learn how a failed attempt moves to the
next provider you open it, scroll past a UI trait, an approver, a cancel token, report types,
three closure aliases and an 11-option builder, and find a 109-line `run_attempts` that calls
ten private methods spread over the next 400 lines, three of which are about orchestration and
two about logging. Its tests start at line 1,145 and run for 1,690 lines in one module.

```
src/turn.rs                                          2,835 lines
  Ui (9 methods: progress + approval, takes formatted text)      49
  EngineApprover                                                112
  Failover, NoRouteInfo, Cancel                                 162
  TurnStatus, TurnReport, Judged                                235
  BackendFactory / GitStatus / Clock closure aliases            298
  git_status_short (spawns git), idle_timeout_from_env          303
  Engine { 22 fields }                                          354
  impl Engine { 50 methods }                                    390-1139
     builder, accessors, sessions, undo, save, resume,
     run, judge, snapshot, run_attempts, choose, orchestration,
     legacy choice, prompt, run_attempt, finish, logging, messages
  mod tests { everything }                                      1145-2835
```

**After.** The folder reads as the steps of a turn. Each file fits on a few screens.

```
crates/turn/
  Cargo.toml              depends on: the crates of its row in 1.3
  README.md               "Carrying one turn from prompt to report."
  src/                    (the entries below are inside src/)
  lib.rs                  pub use: Engine, EngineBuilder, TurnObserver, TurnReport, TurnStatus,
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

Every line is a command or a file check, and all of them except the two marked "person" are
run by `scripts/dod.sh` (added in P5-6), which exits non-zero on the first failure.

**Readability**

- [ ] `python3 -I scripts/budget.py` exits 0: no `.rs` file over 400 lines (test-only files
      600), no function over 60 lines, no binary root over 30 lines, no folder over 20 files,
      no `lib.rs` exporting more than 40 names, no member `Cargo.toml` over 25 lines.
- [ ] Every crate folder has a `README.md` with the template's sections, and every module
      starts with a `//!` "Owns / Must not" comment (check A11).
- [ ] No file or module is named `util`, `utils`, `helpers`, `misc`, `common`, `types`,
      `manager`, `stuff` (check A12).
- [ ] Every `lib.rs` and `mod.rs` contains only docs, attributes, `mod` items and explicit
      `pub use` lists (check A10).
- [ ] (person) The ten questions of 9.1 are answered correctly from the tree and READMEs alone
      by someone who did not do the refactor, or by a fresh agent given only `ls -R crates` and
      the READMEs: ten of ten, each in under a minute.
- [ ] Each overloaded name of 1.1 has one definition:
      `grep -rnE '(struct|enum|trait) (Decision|Gate|Snapshot|SessionInfo|Usage|Interrupt)\b' crates`
      and `grep -rn 'fn run_turn\b' crates` print one line per name.

**Architecture**

- [ ] `ls crates` lists exactly 19 folders (the 16 contexts, `embed`, `testkit`, `cli`), and
      `cargo metadata --no-deps` lists each as a workspace member (`jev-<context>`, `jev-embed`,
      `jev-testkit`, `jev-router`); the root manifest has no `[package]` and has the three
      workspace lines of 4.3 (check C1). Flashboard's crates, if present, are extra members
      outside `crates/` and outside `default-members`.
- [ ] `tests/architecture_baseline/` does not exist and `cargo test --test architecture`
      passes (checks A1, A4-A14, C1-C3; T1 is gone with the flat modules).
- [ ] `cargo test -p jev-embed` passes: `tests/embed_api.rs` pins the six contract names of
      4.4 with their signatures and the chat-window names; `crates/embed/src/lib.rs` contains
      only docs, attributes and `pub use` lines. With Flashboard present,
      `cargo test -p jev-usage-io --test engine_bridge` passes.
- [ ] No compatibility shim is left: `find crates -name legacy.rs` and
      `grep -rnE 'mod legacy\b|::legacy::' crates --include=*.rs` print nothing.
- [ ] The duplicated definitions of 7.2 are gone: `grep -rnE 'fn (write_atomic|write_private_atomic|write_private|env_f64|env_or|lookup_f64|human_ago|format_left|truncate_chars|truncate_lines|truncate_with_note|last_chars|git_status_short|in_git_repo|is_harness)\b' crates`
      prints at most one line per name, and
      `grep -rnE 'unwrap_or_else\((PoisonError::into_inner|\|e\| e\.into_inner\(\))' crates`
      prints one line (in `shared/domain/lock.rs`; both spellings are in use today).

**Type strictness**

- [ ] `cargo clippy --all-targets -- -D warnings` passes with the lint tables of 5.6, and no
      attribute switches one of them off outside the places 5.6 names (check A14).
- [ ] `serde_json::Value`, `serde_json::Map` and `json!` appear in production code only in the
      three files of 5.2, each under 150 lines with no `pub` signature naming them (check A6).
- [ ] `unsafe` appears only in `crates/shared/src/infrastructure/sys_unix.rs`, every block has
      a `// SAFETY:` comment, and every other crate root and every binary has
      `#![forbid(unsafe_code)]` (check A7).
- [ ] No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `as` cast, `dyn Any`, downcast,
      `Box<dyn Error>` or `Result<_, String>` in production code (the clippy tables for the
      first six, check A13 for the last four).
- [ ] `anyhow` and `libc` each appear in one member manifest:
      `grep -ln anyhow crates/*/Cargo.toml` prints `crates/cli/Cargo.toml`,
      `grep -ln libc crates/*/Cargo.toml` prints `crates/shared/Cargo.toml`.
- [ ] `std::env` is read only in `crates/shared/src/infrastructure/env.rs` (check A8);
      `Secret::expose` is called only in `infrastructure/` (check A9).

**No behaviour change**

- [ ] `scripts/check.sh` passes locally and the CI run of the final commit is green: all Rust
      tests (at least the baseline count), `scripts/test_pty_ux.py` (35 scenarios), the
      three Python test files.
- [ ] `git diff ddd-base -- tests/golden tests/fixtures` is empty and the golden tests pass.
- [ ] `scripts/parity.py ddd-start` prints no difference.
- [ ] `scripts/install.sh "$tmp"` installs `jev-code`, `jev-ask`, `jev-delegate-mcp`,
      `jev-chat`, `jev-gateway`, `jev-claude`; `"$tmp/jev-code" --help` runs;
      `git diff ddd-start -- scripts/install.sh scripts/jev-claude mods` is empty.
- [ ] The eight `[[bin]]` names are unchanged
      (`cargo metadata` bin targets: `ask chat code delegate_mcp gateway orch_tune route tune`)
      and no outside dependency was added or re-versioned: the `[[package]]` entries of
      `Cargo.lock` with a `source` are identical to `ddd-start`.
- [ ] `git log --follow --oneline` on `crates/turn/src/application/engine.rs`,
      `crates/orchestration/src/application/delegator.rs`,
      `crates/backend/src/infrastructure/process/child.rs`,
      `crates/changes/src/infrastructure/git_working_tree.rs` and
      `crates/approval/src/domain/rule_policy.rs` shows commits older than `ddd-base`.
- [ ] No secret was added (from `ddd-start`, so the goldens written in P0-3 are scanned too):
      `git diff ddd-start HEAD | grep -nE '^\+.*(sk-ant-|sk-[A-Za-z0-9]{20,}|AIza[0-9A-Za-z_-]{20,}|_API_KEY=[^ "$]{8,})'`
      prints nothing (goldens and fixtures included).
- [ ] (person) A final review round over the whole diff finds nothing.

---

## 11. Decisions for the user

| #   | Decision                                                                                                                 | Status                                                                                                                      | What follows                                                                                                                                                                                                                                                                              |
| --- | ------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| U1  | One crate with a test-enforced rule, or one crate per context                                                            | **Decided: one crate per context**, a Cargo workspace (3.2)                                                                 | The compiler and Cargo enforce the context map. Costs accepted: 19 manifests, `pub` surfaces, the `testing` feature for cross-crate fakes, an untangling phase (P1a) before extraction. Flashboard is unaffected: its two engine lines name `jev-embed` (4.4)                             |
| U2  | Scope: structure only (phases 0-3 and 5), or structure plus type strictness (phase 4)                                    | Taken as recommended: both, in this order                                                                                   | Stopping after phase 3 would leave `Value` and `json!` on about 400 production lines                                                                                                                                                                                                      |
| U3  | The three allow-listed wire files of 5.2                                                                                 | **Decided: accepted**                                                                                                       | They stay minimal and tested: under 150 lines each, no `Value` in a `pub` signature, a golden round-trip test each (check A6). A fourth file needs a new decision                                                                                                                         |
| U4  | CI                                                                                                                       | **Decided: yes**                                                                                                            | `.github/workflows/check.yml` runs `scripts/check.sh` on push and pull request in `putraridho/jev-switch`: no secrets, no provider CLIs, no Jev calls (3.6)                                                                                                                               |
| U5  | Formatting                                                                                                               | Taken as recommended: no reformat during the refactor                                                                       | One `rustfmt` commit after `ddd-done`, listed in `.git-blame-ignore-revs`, is recommended; formatting during the moves would break `git log --follow` and `move_check.py`                                                                                                                 |
| U6  | Glossary renames in P5-2 (`Runtime` to `ProviderRegistry`, `ChangeSummary` to `ChangeSet`, `ProviderWant` to `Judgment`) | Taken as recommended: do them                                                                                               | Compiler-checked; the names appear in no file format. The six names the facade exports to Flashboard keep their names (4.4)                                                                                                                                                               |
| U7  | Budgets                                                                                                                  | Taken as recommended: 400 lines per file, 60 per function                                                                   | The export budget is 40 names per crate, not 25: `backend` exports its types, ports, adapters and the CLI facts the probes read                                                                                                                                                           |
| U8  | Timing                                                                                                                   | Taken as recommended: merge all open branches first, then freeze feature work from P1a to `ddd-p1`                          | P1a and P1b touch every file. Flashboard's first task may run before or after the refactor; 4.4 covers both orders                                                                                                                                                                        |
| U9  | The package that holds the binaries keeps the name `jev-router` (`crates/cli`), and the repository folder keeps its name | Taken as recommended (default): keep                                                                                        | Nothing a user sees depends on it. Facts for keeping: `Cargo.lock` stays free of a renamed package (the P1b-0 check), and a Flashboard built before the refactor names `package = "jev-router"` (4.4). No fact argues for a rename                                                        |
| U10 | Pin the toolchain in `rust-toolchain.toml`                                                                               | Taken as recommended (default): `channel = "1.95.0"`, `components = ["clippy"]`, raised deliberately in a commit of its own | CI and every developer use one clippy, so a new release cannot turn an old green commit red. Its nightly-only fuzz crate is outside the workspace and asks for `+nightly` explicitly, which a toolchain file does not override |

U2 and U5 to U10 are defaults taken from this document's recommendation; the user can still
change any of them before P0 starts.
