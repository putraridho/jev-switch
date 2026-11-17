# Flashboard (the Jev usage dashboard): work breakdown

Read `DESIGN.md` first. It is normative; this file says who builds which part of it.
Section numbers below ("D4.2") refer to `DESIGN.md`. D1a (the user's decisions) overrides
everything, including this file.

## 0. Rules for every worker

1. **Ownership is exclusive.** Edit only the files your task owns. Every file that more than
   one task touches or imports (manifests, module lists, type files, every Rust source file,
   the UI's shared seams, fixtures) is created by T00, as a compiling stub with its final
   public signatures, and then owned by exactly one later task. Files private to one task
   (its tests, and the directories listed with `**` under its "Owns") are created by that
   task. If you need a change in a file you do not own, or in a **frozen** file (list
   below), stop and send the lead a contract change request; do not edit it.
2. **Frozen after T00** (lead approval to change): every `Cargo.toml`, `Cargo.lock`,
   `deny.toml`, every `lib.rs`/`mod.rs`, `src/embed.rs` (Path A, D5), `engine_bridge.rs` and
   its dependency line (D5.1), `dashboard/package.json`,
   `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`, `vitest.config.ts`,
   `playwright.config.ts`, lint configs,
   `dashboard/src-tauri/{build.rs,tauri.conf.json,capabilities/}`, the core type files
   `time.rs quantity.rs label.rs provider.rs meter.rs usage.rs port.rs secret.rs ipc.rs`,
   `dashboard/src/lib/ipc/bindings/`, `dashboard/src/lib/ipc/client.ts` (the `IpcClient`
   interface), everything under `dashboard/fixtures/` (shared by Rust and TypeScript tests;
   a wrong vector is a contract change). Type
   definitions inside a file you take over are also frozen; you write function bodies and
   private items.
3. **No new dependency** (Rust or npm) without lead approval. T00 declares all of them.
4. **Never** run a model prompt through any provider CLI, never run `grok`, `agy` or
   `gemini`, never open `gateway.jsonl`, `turns.jsonl`, `code-turns.jsonl`, `history`,
   `sessions/` or any CLI's token file, never print or paste real log lines, e-mails or ids
   into tests, fixtures, commits or reports. Fixtures are synthetic or the redacted ones in
   `dashboard/fixtures/`.
5. Tests never touch the real home directory, the real state dir, the network or real CLIs:
   temp dirs, stub programs, a local stub HTTP server, `FakeClock`, tokio paused time.
   No test calls `std::env::set_var`/`remove_var` (a race in the parallel harness, and
   `unsafe` from edition 2024): code that reads the environment has a pure `_from` variant
   taking the values, and the few real-environment cases re-execute the test binary as a
   child with `Command::env` (D5.1). Tests may use `std::process::Command` for that and
   for stubs; production code only in `proc.rs`.
6. Lints are `-D warnings`. No `unwrap`/`expect`/`panic!`/indexing/`as` outside tests, no
   `println!`/`eprintln!`/`dbg!`, no `unsafe`, no error type that carries a string, no
   `TODO`, no `serde_json::Value` or `json!` outside tests, `#![forbid(unsafe_code)]` in all
   three crates. A lint `allow` needs a one-line reason and lead approval; the only two
   approved in advance are the fixed test header and the one float cast of D11.1, used
   exactly as written there. No `pub` signature carries a foreign error type
   (`std::io::Error` included, D8.4).
   **TypeScript (D1a, D10 "Type discipline")**: no `any`, `unknown` or `never`, no type
   assertion (`as const` and `satisfies` are fine), no non-null `!`, no `@ts-` directive, no
   `Function`/`Object`/`object`/`{}` type, no `catch` binding and no parameter on a rejection
   callback, no type predicate (`x is T`, `asserts`), no `declare`, no definite `!`, no
   `JSON.parse`, no value whose inferred type is `any`, no `.js` file, every component
   `lang="ts"` with typed props, exhaustiveness by `satisfies Record<…>` tables. This holds
   for tests, e2e specs and config files too. There is no approved exception and inline
   ESLint configuration is ignored; if you believe you need one, stop and ask the lead.
7. Done means D11.5, including its rule that review rounds repeat until one finds nothing.
   Report with: files changed, tests added (names), how each security rule of the task is
   proven, anything you could not verify, and every assumption of D16 you confirmed or
   found false.
8. The engine (its sources, root tests, `scripts/install.sh`) must stay green and untouched.
   The engine may be refactored into per-context crates with a facade crate before, during
   or after this project (D5): in **Path A** (not yet refactored when T00 starts) T00 adds
   `src/embed.rs`, one `pub mod embed;` line and the word `pub` on
   `availability::parse_codex_login`, and nothing else; in **Path B** nobody here edits an
   engine file. Either way only `usage-io/src/engine_bridge.rs` names the engine, and no
   dashboard code reaches `probe_all` or `path_from_env`.
9. Names (D1a): the product is **Flashboard**, the identifier `dev.jev.flashboard`, the
   binary and crates `jev-dashboard`/`jev-usage-*`. No file, string or test says
   "Jev Dashboard" or `dev.jev.dashboard`.
10. UI tasks (T07, T09, T10, T13, T14): D9.1 and D9.10 are acceptance criteria, not guidance.
    Minimal chrome and palette, the three disclosure levels, and every accessibility item
    are checked by the lead; a UI task with an open accessibility finding is not done.
11. This machine's `codex` and `claude` are self-updating mise wrappers (D3.1). Never run
    either outside the rules of D8.6, and never start the installed Flashboard's session
    name: every launch of the real binary in a test goes through `dbus-run-session` (D8.8).
    The account wrapper behind them can run `mise install` or rewrite the user's mise
    configuration (D3.1). So **a worker never starts the app against the real `PATH`**:
    `pnpm dev` and every hand launch use the stub `codex`/`claude` directory and a temp
    `HOME` (the script `dev` sets both; T00 ships two minimal stubs, T12 replaces them with
    the smoke test's). The real-`PATH` launch (`pnpm dev:real`) is for the lead, and only
    after T04's manual check has confirmed the D8.6 assumptions on this machine.

## 1. DAG and waves

```
W0   T00 scaffold + contract
      │
W1   T01 parsers   T02 policy   T03 claude log   T04 subprocess + login probe   T05 deepseek
     T06 persistence   T07 UI foundation   T14 UI primitives   T15 shell leaf modules
      │
W2   T08 usage service (needs T01–T06)        T09 overview   T10 detail + settings   (need T07, T14)
      │
W3   T11 Tauri shell wiring (needs T08, T15; UI contract from T07)
      │
W4   T12 security proofs + CI + e2e + budgets        T13 packaging + UI polish audit
```
All tasks in a wave run in parallel, each in its own worktree. A wave starts when the
previous wave is merged and `scripts/ci.sh` is green. Inside wave 1 there are soft orders:
T03, T04, T05 call T01's parsers, T06 calls T02's history rules, T15's notifier calls T02's
`text`, and T14's primitives use T07's tokens, stores and Playwright harness. They compile
against T00's stubs from the start, so nobody waits; the lead merges T01, T02 and T07 first,
and the others rebase on them before their final test run (their fixture-driven and
screenshot tests cannot pass against the neutral stubs). Inside wave 4 there is the same kind of soft order: T13's polish pass
regenerates screenshot baselines, so the lead merges T13 first and T12 makes its final
`scripts/ci.sh` run on top of it.

| task | wave | UI-design pass | skills to load |
|---|---|---|---|
| T00 | 0 | no (creates token file skeleton only) | none |
| T01–T06, T08, T15 | 1–2 | no (T15: tray icon only, follow D9.11 exactly) | none |
| T07 | 1 | **yes, primary** (tokens, base, app shell) | `frontend-design` then `impeccable` |
| T14 | 1 | **yes** (primitives) | `impeccable`, finish with `better-ui` |
| T09 | 2 | **yes** | `impeccable`, finish with `better-ui` |
| T10 | 2 | **yes** | `impeccable`, `dataviz` for the chart, finish with `better-ui` |
| T11 | 3 | no | none |
| T12 | 4 | no | `security-review` on the final diff |
| T13 | 4 | **yes, audit** | `impeccable` (audit + polish), `better-ui` |

`artifact-design` is not used anywhere in this project.

---

## T00 — Scaffold and shared contract (wave 0, one worker, strongest model)

**Goal.** Create the workspace, every crate, every file, the complete domain and IPC types,
the engine facade, the generated TypeScript bindings, the UI skeleton, the fixtures and a
first `scripts/ci.sh`, so that everything compiles and every later task only fills bodies.

**Owns (creates).**
- Root `Cargo.toml`, per the path of D5 (say which in the report). Path A: the
  `[workspace]` table (with `exclude = [".claude", "dashboard/fuzz"]`) and `[profile.dist]`.
  Path B: only the three members, the two `exclude` entries and `[profile.dist]` are added
  to the existing workspace, plus `default-members = ["crates/*"]` **when the refactor's
  root has no `default-members` key** (a virtual root without one makes every member a
  default member, which would break D5 invariant 1); an existing key is left alone.
  Never `[workspace.lints]`; no `panic = "abort"`. Also `Cargo.lock`, `deny.toml`, `dashboard/clippy.toml`
  (D11.1; not at the root), the same `[lints]` table in each of the three app manifests
  (D5 invariant 3), and **in Path A only** `src/embed.rs`, one line `pub mod embed;` in
  `src/lib.rs` and the word `pub` on `parse_codex_login` in `src/availability.rs`,
  `scripts/ci.sh` (stages 1–9 of D11.4 working, so every wave-1 to wave-3 task, the UI tasks'
  Playwright specs included, is gated from its first merge; stages 10–13 present and
  reporting SKIPPED with a reason until T12), `.gitignore` additions.
- `dashboard/crates/usage-core/**`: manifests, `lib.rs`, the frozen type files **complete
  with tests** (`time quantity label provider meter usage port secret ipc`), and stubs for
  `parse/{claude_log,claude_event,claude_auth,codex,deepseek}.rs`, `assess.rs alerts.rs summary.rs
  history.rs text.rs settings.rs view.rs` (types complete, functions returning a neutral
  value: `None`, `Err(Unavailable::Internal)`, an empty vec; no `todo!`, no lint allows).
- `dashboard/crates/usage-io/**`: manifests (with the one engine dependency line, renamed
  to `jev-engine`, D5.1), `lib.rs`, **complete** `engine_bridge.rs` (the engine port, with
  its contract tests in `tests/engine_bridge.rs`), `envfile.rs`, `clock.rs`, `diag.rs`; stubs for `fsutil tail watch proc probe http secrets
  store settings_file timer registry service` and `sources/{claude_log,claude_live,codex,deepseek,placeholder}.rs`
  (`placeholder.rs` complete).
- `dashboard/src-tauri/**`: `Cargo.toml` (a library target plus the binary, so the shell's
  modules can be tested from `tests/`), `build.rs` (AppManifest commands, D8.8),
  `tauri.conf.json` (CSP, security flags, identifier `dev.jev.flashboard`, product name
  `Flashboard`, `app.windows: []`, no bundle targets), `capabilities/main.json`, `lib.rs`
  (the module list and `pub fn run()`), a three-line `main.rs`, an `app.rs` that creates the
  `main` window in code and shows it empty, stub modules `commands events tray tray_icon
  notifier window autostart hardening localtime bus cli`, and a placeholder `icons/` set
  (T13 replaces it).
- `dashboard/` UI: `package.json` (exact versions, scripts `dev dev:real build check lint
  test test:screens tauri`; `dev` passes the identifier `dev.jev.flashboard.dev`, D8.8, and
  runs against stub CLIs and a temp home, rule 11; two stub programs under
  `dashboard/e2e/smoke/stubs/` and the activation-free bus configuration
  `dashboard/e2e/smoke/session-bus.conf` (D8.8), all later T12's),
  `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`
  (`build.assetsInlineLimit: 0`), `vitest.config.ts`, `playwright.config.ts` (Chromium
  project; the static server sends the CSP of `tauri.conf.json` as a header),
  ESLint/stylelint/Prettier configs with the rules of D8.10 (raw-HTML sinks, web storage,
  `style=` attributes), the import rule for `.svelte` files (D10), D9.7 and the complete **type discipline of D10** (`tsconfig.json`
  flags, the type-checked ESLint rule set run with `--no-inline-config --max-warnings 0`,
  and `dashboard/scripts/check-types.ts`, all frozen; config files are TypeScript, or JSON
  for stylelint and Prettier; no `svelte.config.js`, D10), `src/env.d.ts`, `index.html` (`lang="en"`), `src/main.ts`, stub
  `src/App.svelte`, stub screens `src/screens/{Overview,ProviderDetail,Settings}.svelte`,
  `src/lib/design/tokens.css` with the token **names** of D9.7 (values are T07's),
  `src/lib/ipc/bindings/*` (generated), `src/lib/ipc/client.ts` (the complete `IpcClient`
  interface of T07, types only), and stubs with the final exported signatures of T07 for
  `src/lib/format/index.ts`, `src/lib/copy.ts`, `src/lib/stores/*.svelte.ts`,
  `src/lib/router.svelte.ts`, `src/lib/boot.ts`, `src/lib/ipc/{tauri,mock,fixtures}.ts` and
  the Playwright harness `e2e/screens/_harness/index.ts` (so T14, T09 and T10 compile
  against them from the first day; without the harness stub T14's spec would not).
- `dashboard/fixtures/`: keep `codex-app-server/` (already recorded), add
  `claude-gateway/{sample.jsonl,canary.jsonl,rotation/…}` (synthetic rows shaped like D3.2,
  both canaries of D8.10), `claude-auth/{logged_in.json,logged_out.json,text_form.txt}`
  (synthetic, key names of D3.1, canary e-mail, org id and org name), `codex-app-server/
  paid_plan.response.json` (synthetic from the schema: 300 and 10080 minutes), `deepseek/{balance_ok.json,balance_two_currencies.json,
  unauthorized.json}`, `format-vectors.json` (≥ 60 cases for D9.9), `ipc-allowlist.txt`.

**Interfaces to expose (exactly as written in the design).**
- D4.1–D4.4 types and ports, D7.4 IPC types (`core::ipc`, including the generic
  `Outcome<T>`), D8.5 `Secret<T>`, D7.8
  `LocalStamp`/`LocalTime` (in `core::text`), D5.1 the engine port `io::engine_bridge`
  (and, in Path A, the facade module behind it).
- Additional types the stubs need, defined here and frozen:
  ```rust
  // core::assess
  pub struct FreshnessPolicy { pub fresh_for: Duration }
  pub struct SourceState { pub last_good: Option<Reading>,
                           pub last_failure: Option<(UnixMillis, SourceError)>, pub attempted: bool }
  pub enum Phase { Current, Elapsed }                                   // IPC
  pub enum Severity { Ok, Warn, Critical, Unknown }                     // IPC
  // core::settings
  pub struct Thresholds { pub warn: Percent, pub critical: Percent }    // invariant warn < critical, private fields + ctor
  pub struct Settings { /* fields of D7.4 `Settings` */ }               // IPC
  pub struct SettingsPatch { /* every field Option<_>, #[serde(default)], #[ts(optional)] */ }   // IPC (Deserialize)
  // core::alerts
  pub enum AlertLevel { Warn, Critical }
  pub struct Alert { pub provider: ProviderKey, pub window: WindowKind, pub level: AlertLevel,
                     pub used: Percent, pub resets_at: Option<UnixSeconds> }
  pub struct AlertEntry { pub provider: ProviderKey, pub meter: MeterId, pub level: AlertLevel,
                          pub fired_at: UnixSeconds, pub window_resets_at: Option<UnixSeconds> }
  pub struct AlertMemory { pub entries: Vec<AlertEntry> }
  // core::history
  pub struct StoredSample { pub at: UnixSeconds, pub value: i64, pub resets_at: Option<UnixSeconds> }
  pub struct SamplePoint { pub at: UnixSeconds, pub value: i64, pub lo: i64, pub hi: i64 }
  pub struct SeriesData { pub points: Vec<SamplePoint>, pub resets: Vec<UnixSeconds>, pub bucket: Duration }
  // core::history (also)
  pub struct StoreStats { pub bytes: u64, pub samples: u64 }           // HistoryStore::stats
  // core::parse::claude_auth (D8.3, D8.6; T01 writes the body)
  pub struct ClaudeLogin { pub logged_in: bool, pub plan: Option<Label> }
  pub fn parse(output: &[u8]) -> Option<ClaudeLogin>;
  // io::probe (types complete here; T04 writes CliProbe's body)
  pub enum Readiness { Ready, NotInstalled, NotLoggedIn, Unknown }
  pub struct LoginFacts { /* private: a Readiness and an Option<Label> per ProviderKey */ }
  impl LoginFacts {
      pub fn unknown() -> Self;                                         // before the first probe: every provider `Unknown`
      pub fn with(self, p: ProviderKey, r: Readiness, plan: Option<Label>) -> Self;
      pub fn readiness(&self, p: ProviderKey) -> Readiness;
      pub fn plan(&self, p: ProviderKey) -> Option<Label>;
  }
  pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = LoginFacts> + Send + 'a>>;
  /// The one seam to the CLIs' login state, so the service is testable. `providers` are the
  /// ones to probe (not hidden, with a `needs_login` source); the others stay `Unknown`.
  pub trait LoginProbe: Send + Sync { fn probe(&self, providers: Vec<ProviderKey>) -> ProbeFuture<'_>; }
  pub struct CliProbe;  impl CliProbe { pub fn new(runner: Arc<ProcRunner>) -> Self; }  impl LoginProbe for CliProbe   // D8.6
  /// feature "test-support": answers in order, repeating the last one, and counts calls.
  pub struct ScriptedProbe;  impl ScriptedProbe { pub fn new(answers: Vec<LoginFacts>) -> Arc<Self>; pub fn calls(&self) -> u32; }
  impl LoginProbe for ScriptedProbe
  pub struct FactsCell(/* RwLock<LoginFacts> */);                      // shared read handle
  impl FactsCell { pub fn new(f: LoginFacts) -> Arc<Self>; pub fn get(&self) -> LoginFacts; pub fn set(&self, f: LoginFacts); }
  // io::envfile (complete here): the only reader of ~/.jev-router.env
  /// Reads the file into a zeroizing buffer, returns `engine_bridge::parse_env(text, name)`, zeroizes the buffer.
  pub fn read_value(path: &Path, name: &str) -> Option<Zeroizing<String>>;
  // io::engine_bridge (complete here): the engine port of D5.1, the ONLY file that names the engine
  pub fn state_dir() -> Option<PathBuf>;
  pub fn parse_env(text: &str, name: &str) -> Option<String>;
  pub fn codex_logged_in(exit_ok: bool, text: &str) -> Option<bool>;
  /// Pure and tested: process variable, else the env-file key, else `<state_dir>/usage.jsonl` (D5.1).
  pub fn usage_log_path_from(env: Option<String>, env_file: Option<&Path>, state_dir: Option<PathBuf>) -> Option<PathBuf>;
  pub fn usage_log_path() -> Option<PathBuf>;                           // the above over the real environment
  pub fn dashboard_dir() -> Option<PathBuf>;                            // <state_dir>/dashboard (debug builds: dashboard-dev, D8.8)
  pub fn env_file() -> Option<PathBuf>;                                 // the facade's env_file_path over the real environment
  /// The engine's login command for a dashboard provider (D4.1); None where the engine has none.
  pub fn login_command(p: ProviderKey) -> Option<&'static str>;
  // io::clock
  pub struct SystemClock;  impl Clock for SystemClock
  pub struct FakeClock;    impl FakeClock { pub fn at(ms: u64) -> Arc<Self>; pub fn advance(&self, d: Duration); pub fn set(&self, ms: u64); }   // feature "test-support"
  // io::diag
  pub enum DiagEvent { SourceOk { source: SourceId, took_ms: u32 }, SourceFailed { source: SourceId, reason: Unavailable },
                       StoreFailed { error: StoreError }, LogRotated, WatchLost, WatchArmed, ClockChanged,
                       ProbeDone { took_ms: u32 }, BackfillDone { samples: u32 }, WindowReady { took_ms: u32 },
                       NotifyFailed, AutostartFailed, Started, Stopping }
  pub fn emit(e: DiagEvent);                                            // stderr, one line, numbers and enum names only
  /// feature "test-support": collect instead of print. Process-wide; the guard holds a
  /// mutex, so tests that capture run one at a time and never see each other's events.
  pub fn capture() -> CaptureGuard;
  // io::fsutil (type only; T06 writes the bodies): the error of every file helper (D8.4)
  #[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum FsError { Denied, Missing, Invalid, Other }
  // io::timer (D7.1)
  pub enum Wake { Due, ClockChanged }
  pub trait WallTimer: Send + Sync { fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_>; }
  pub struct SystemTimer;  pub struct FakeTimer;                         // FakeTimer: feature "test-support"
  // io::proc (types only; T04 writes the bodies): see T04 for `Program`, `ProcSpec`, `Stderr`, `Exit`, `Resolved`, `ProcRunner`
  // io::service (types only; T08 writes the bodies)
  pub trait Jitter: Send + Sync { fn next(&self) -> f64; }
  pub struct Backfill { pub provider: ProviderKey, pub run: Box<dyn FnOnce() -> Result<u64, StoreError> + Send> }
  pub struct ShellFacts { pub app_version: &'static str, pub tray_available: bool }
  // io::store (types only; T06 writes the bodies)
  pub enum MetaKey { BackfillDone, LastPruneAt, WindowWidth, WindowHeight }
  ```
- Every type shown under a later task's "Exposes" is declared by T00 in that task's stub
  file, exactly as written there; the block above lists only what no later task shows.
- `usage-io` has a Cargo feature `test-support` (FakeClock, FakeTimer, ScriptedProbe, diag
  capture, proc/http test constructors) enabled only by dev-dependencies and tests.
- IPC names: `ProviderKey` serialises exactly as `as_str()` (`deepseek`, not `deep_seek`);
  `Setup` has two variants (`use_gateway`, `add_deepseek_key`), D4.3. `MeterId` and `Label`
  have hand-written serde impls (key string, validated on the way in); `WindowKind::Named`
  and `StaleCause::Failing` are struct variants (D4.2); argument types (`RefreshTarget`,
  `HistoryQuery`, `SettingsPatch`) and `Settings` also derive `Deserialize`. A round-trip
  test serialises one value of every IPC enum variant (this is where a tagged newtype
  variant would fail at run time). `#[ts(type = …)]` replaces the whole field type, so
  `Option` and `Vec` fields spell the wrapper (`"number | null"`, `"string | null"`,
  `"number[]"`, D7.4); a test serialises values with every optional field `None` and
  checks that each key that came out `null` is typed `| null` in the generated file.
- Bindings: `cargo test -p jev-usage-core --features bindings export_bindings` writes
  `dashboard/src/lib/ipc/bindings/*.ts`; `ci.sh` fails on drift.

**Acceptance.**
- `cargo build --all-targets --locked` and `cargo test --locked` at the root: unchanged
  behaviour, all existing engine tests pass; `scripts/install.sh` untouched and working.
- `pnpm --dir dashboard install --frozen-lockfile && pnpm --dir dashboard build && pnpm
  --dir dashboard check && pnpm --dir dashboard lint` pass; then (the shell embeds the built
  UI, D11.4 stage 2) `cargo test -p jev-usage-core -p jev-usage-io --locked` and `cargo
  build -p jev-dashboard --locked` pass; `cargo fmt --check` and clippy pass **scoped to
  the three app crates** (D11.1);
  `pnpm --dir dashboard tauri build --no-bundle -- --profile dist` produces a binary that
  opens an empty window.
- Unit and property tests for the frozen types: `Percent` (saturation, NaN/negative
  rejected, ordering), `Money::parse_decimal` (exact, rejects exponent/garbage/overflow,
  round-trip), `Label::parse` (charset, length 1..=40), `Meters::new` (rejects empty, > 8,
  duplicate ids), `MeterId::key`/`parse_key` round-trip for arbitrary values,
  `WindowKind::from_minutes`.
- `static_assertions`: `Secret<String>` implements none of `Debug Display Clone Serialize
  PartialEq`. The **type-file test** of D8.10 (`usage-core/tests/type_files.rs`, `syn`):
  no struct field or enum variant field in the frozen type files, or in any other core
  type that derives `TS` (`Settings`, `SettingsPatch`, `Severity`, `Phase`: IPC types
  defined beside their logic and re-exported from `ipc`, D7.4), has a string-like type,
  except `Label`'s `Box<str>` and `Diagnostics.app_version`.
- `LoginFacts`: `unknown()` is `Unknown` everywhere; `with` sets one provider; Debug output
  holds only readiness names and labels.
- `usage_log_path_from`: the three sources in order; equal to the engine's
  `log::path_from_env("JEV_USAGE_LOG", "usage.jsonl")` rule for the same inputs (table test
  over a temp env file; the engine function itself is not called). `envfile::read_value`:
  `export`, quotes, comments, missing file, missing key.
- **Engine port** (D5.1): the contract tests of `tests/engine_bridge.rs` pass (env-file
  cases, the Codex login truth table, the two login command strings, state directory and
  env-file path under a temp environment, in a child process, rule 5); `jev_engine`/`jev_router` occur in no dashboard
  `.rs` file except `engine_bridge.rs` and in no manifest except `usage-io`'s (a grep test
  here; T12's architecture test takes it over); `usage-core` and the shell do not depend
  on the engine (`cargo tree -p jev-usage-core -i jev-engine` finds nothing). Report the
  path taken and, in Path B, any of the six facade names that was missing (a request to
  the engine's owner, not an edit; `parse_codex_login` is known to be missing from the
  plan's list), and the two other requests of D5: the engine's `--workspace` gates
  exclude the three app crates (invariant 5) and its manifest check exempts them
  (invariant 3).
- **Type discipline** (D1a): `#![forbid(unsafe_code)]` is the first line of all three crate
  roots and the Tauri scaffold compiles under it (report if a Tauri macro does not);
  `check-types.ts` passes over the stubs, the configs and the generated bindings, and a
  self-test feeds it one file per banned construct (`any`, `unknown`, `never`, `x as T`,
  `<T>x`, `x!`, `let x!: T`, `// @ts-ignore`, `catch (e)`, `.catch((e) => …)`, a type
  predicate, a `declare const`, `JSON.parse`, `querySelector<T>`, a value inferred as `any`
  from a third-party type, an `as` inside the markup of a `.svelte` file, a second
  `svelte-ignore`, a `<script>` without `lang="ts"`, a tracked `.js` file)
  and sees each one reported, while `as const`, `satisfies` and the word "never" in a string
  pass; ESLint reports the same constructs.
- Lints (D11.1): the three app manifests carry an identical `[lints]` table (compared by
  a test) and none says `lints.workspace`; `dashboard/clippy.toml` is picked up by the three app crates and not by
  the engine (a `serde_json::Value` in a stub fails clippy; the engine's own use does not);
  the fixed test header and the single float cast are the only `allow`s in the tree.
- `scripts/ci.sh` runs and prints the stage table.
- Workspace: `cargo metadata` succeeds from inside a pre-existing agent worktree under
  `.claude/worktrees/` (the `exclude` works), and `cargo build` at the root builds only the
  engine.
- Confirm and report (D16): the pinned versions of Tauri 2, Svelte 5, Vite; that `tauri
  build -- --profile dist` lands in `target/dist/`; whether `tauri-plugin-notification` has an
  urgency setting (if it does, tell the lead before using `notify-rust`); that the scaffold
  window renders a Svelte component with `freezePrototype: true`; that `tauri.conf.json`
  needs the placeholder icons or not; that Tauri's macros pass `forbid(unsafe_code)` and
  `disallowed_types`; that `svelte-check` and ESLint work with no `svelte.config.js`;
  whether `pnpm dev` works under the production `csp` (else `devCsp`, D8.8); the
  final npm dev-dependency list against D8.9; whether two major versions of `zbus` resolve.
- Idle floor: the empty scaffold, started hidden under `dbus-run-session` (with the bus
  configuration of D8.8; report whether GTK and WebKitGTK start on it) for the D11.3
  window, has its CPU time, wakeups and RSS reported (by hand here; T12 scripts it).

**Security rules.** D8.4 (no string in errors, `diag` typed), D8.5, D8.8 (capabilities, CSP,
plugin list exactly as designed), D8.9 (exact pins, lockfiles, `deny.toml`, npm script
blocking and release-age rule), fixtures contain no real data.

**Depends on.** nothing.

**Review checklist (lead).**
- [ ] Root `Cargo.toml` diff is only what D5 allows for the path taken; no
      `[workspace.lints]`; engine build and tests identical.
- [ ] The app crates are members and not default members (`cargo metadata`:
      `workspace_default_members` names none of the three, in either path); `exclude` has `.claude` and
      `dashboard/fuzz`; `profile.dist` separate from `release`.
- [ ] The engine is named in two lines only (D5.1); Path A: `src/embed.rs` is re-exports
      only and the only other engine diff is one `pub`; Path B: no engine diff at all.
- [ ] Every IPC type matches D7.4 field for field; bindings generated, not hand-edited, and
      free of `any`/`unknown`/`never`; `tsconfig.json` and the ESLint rule set match D10.
- [ ] No type in core can hold a free string except `Label`.
- [ ] Direct dependency lists match D8.9 exactly; versions exact; no git sources.
- [ ] `tauri.conf.json`, capability file and `build.rs` match D8.8 character for character.
- [ ] Stubs compile without `todo!`/`unimplemented!`; signatures match this file.

---

## T01 — Parsers (wave 1)

**Goal.** The five allow-list parsers of `jev-usage-core::parse`, proven unable to surface
anything but numbers, closed enums and `Label`s.

**Owns.** `dashboard/crates/usage-core/src/parse/{claude_log,claude_event,claude_auth,codex,deepseek}.rs`,
`dashboard/crates/usage-core/tests/parse_*.rs`.

**Exposes.**
```rust
// parse::claude_log
pub struct ClaudeLogSample { pub at: UnixMillis, pub windows: Meters }      // ≥ 1 window meter
/// Cheap byte test used before JSON parsing: could this line contain a sample?
pub fn might_have_sample(line: &[u8]) -> bool;
/// One line of usage.jsonl. `None` for anything that is not a row with at least one
/// `anthropic-ratelimit-unified-<window>-utilization` value. Never panics on any input.
pub fn parse_line(line: &[u8]) -> Option<ClaudeLogSample>;
// parse::claude_event
/// One line of `claude -p --output-format stream-json`: `Some` only for a rate_limit_event.
pub fn parse_line(line: &[u8]) -> Option<Meters>;
// parse::codex
pub struct CodexLimits { pub meters: Meters, pub plan: Option<Label> }
/// `None` when `line` is not the JSON-RPC response with this id (notifications, other ids).
/// `Some(Err(Protocol))` when it is that response but an error or malformed.
pub fn parse_rate_limits_response(line: &[u8], id: u64) -> Option<Result<CodexLimits, Unavailable>>;
pub fn is_response(line: &[u8], id: u64) -> bool;                            // for the initialize ack
// parse::deepseek
pub fn parse_balance(body: &[u8]) -> Result<Meters, Unavailable>;
// parse::claude_auth — the whole output of `claude auth status` (D8.3, D8.6)
pub struct ClaudeLogin { pub logged_in: bool, pub plan: Option<Label> }
pub fn parse(output: &[u8]) -> Option<ClaudeLogin>;
```
Mapping rules: D6.1 (including the exact **key rule**: split at the last `-`, attributes
`utilization`/`reset`/`status` only), D6.3, D6.4, D3.1, D3.2. Details: utilization and reset
are strings in the log and numbers in the event; a window with a utilization but an
unparsable reset keeps `resets_at: None`; statuses `allowed` → `Allowed`, `allowed_warning`
→ `Warning`, `rejected` → `Rejected`, anything else → `Unknown`; a window name that fails
`Label::parse` drops that window; more than 8 windows keeps the first 8 in key order;
duplicate windows keep the first. `ClaudeLogSample` holds the windows of **one row**;
carrying windows across rows is the source's job (T03). Codex: `usedPercent` is any JSON
number; a JSON-RPC `error` is `Some(Err(ServerError))`, a malformed result
`Some(Err(Protocol))`; only `rateLimits` is read, never `rateLimitsByLimitId`.
`might_have_sample` tests for the bytes `"ratelimit":{"`. `claude_auth::parse`: a serde struct
with exactly `loggedIn`, `planTier`, `subscriptionType`; plan = `planTier`, else
`subscriptionType`, ASCII-lower-cased with `-` and space as `_`, through `Label::parse`;
when the bytes are not that JSON, the text rule of D8.6 on bytes; anything else `None`.

**Acceptance tests.**
- Each parser against its recorded/synthetic fixtures with exact expected `Meters`
  (`rate_limits_read.response.json` → one `Monthly` window, 400 bp, reset 1797439066,
  plan `free`; notifications and other ids → `None`).
- Paid-plan shape (synthetic from the schema): 300 and 10080 minutes → `FiveHour`, `SevenDay`.
- Real key set of D3.2 in one synthetic row (`5h-surpassed-threshold`, `fallback`,
  `fallback-percentage`, `overage-status`, `representative-claim`, `upgrade-paths`, `status`,
  `reset`, `7d_oi-*`): exactly the meters `FiveHour`, `SevenDay`, `Named(7d_oi)`; status
  `allowed_warning` → `Warning`.
- Codex edge cases: `usedPercent` 4, 4.5, 100, 140 and -1; `resetsAt` null; `secondary`
  null; both windows the same duration; `hasCredits` true with a decimal balance, with
  `unlimited`, with a null balance; no window and no balance → `Protocol`.
- `claude_auth`: the three fixtures (logged in with `planTier`, with only
  `subscriptionType`, logged out, the text form) → the exact `ClaudeLogin`; the canary
  e-mail, org id and org name of the fixture are absent from the result's Debug output.
- **Canary**: `claude-gateway/canary.jsonl`, `claude-auth/*` and `account_read.response.json`
  fed to every parser: `format!("{:?}")` of every result contains neither `JEVCANARY` nor
  `canary-email`. Property test: arbitrary JSON with the canary string inserted at random
  keys/values/depths, same assertion, for all five parsers. The **Label-shaped canary**
  (`jevcanary_9f3c1b7e`, D8.10) in every position that is not a label position of D8.3 never
  surfaces; one test documents that it does surface as a window name or a plan.
- Property: no panic and bounded output for arbitrary bytes (also invalid UTF-8, deep
  nesting 10 000 levels, 1 MiB strings, huge numbers, NaN-like tokens, duplicate keys).
- Property: a generated valid sample serialised to a log row with random extra keys parses
  back to the same sample.
- `might_have_sample` has no false negatives (property: `parse_line(l).is_some()` implies
  `might_have_sample(l)`; true by construction, because `parse_line` applies the filter
  first, D6.1: a pretty-printed row is `None`, and one test documents that).

**Security rules.** D8.3 (allow-list visitor; no `serde_json::Value` of a whole row; no
`deny_unknown_fields`; no string retained except through `Label::parse`), D8.4.

**Depends on.** T00.

**Review checklist.**
- [ ] No `Value`, `String`, `&str` field in any deserialisation struct except where it is
      converted in the same function to a number/enum/`Label`.
- [ ] The `ratelimit` visitor ignores non-prefixed keys without allocating their values.
- [ ] Recursion depth is bounded (serde_json default limit kept).
- [ ] Canary and arbitrary-bytes property tests exist and run at CI case counts.
- [ ] Float handling: NaN/inf/negative rejected through `Percent::from_fraction`.

---

## T02 — Policy: freshness, alerts, headline, history rules, text, settings, views (wave 1)

**Goal.** Every decision the app makes, as pure functions.

**Owns.** `dashboard/crates/usage-core/src/{assess,alerts,summary,history,text,settings,view}.rs`,
`dashboard/crates/usage-core/tests/policy_*.rs`.

**Exposes.**
```rust
// assess
pub fn derive(state: &SourceState, now: UnixMillis, policy: &FreshnessPolicy) -> Usage;
pub fn phase(w: &WindowReading, now: UnixMillis) -> Phase;
pub fn severity(m: &Meter, now: UnixMillis, t: &Thresholds) -> Severity;   // D4.5: Unknown when elapsed; balance by `usable`
// alerts (D7.6)
pub fn step(memory: &AlertMemory, provider: ProviderKey, reading: &Reading, now: UnixMillis,
            settings: &Settings) -> (AlertMemory, Vec<Alert>);
// summary
pub struct Headline { pub provider: ProviderKey, pub meter: MeterId, pub used: Percent,
                      pub severity: Severity, pub resets_at: Option<UnixSeconds> }
pub fn headline(providers: &[(ProviderKey, &Usage)], now: UnixMillis, settings: &Settings) -> Option<Headline>;   // the domain value; view::snapshot maps it to ipc::Headline
// history
pub fn should_store(last: Option<&StoredSample>, candidate: &StoredSample) -> bool;   // D7.5 write rule (a candidate not newer than `last` is dropped)
pub fn downsample(points: Vec<SamplePoint>, max: u16) -> Vec<SamplePoint>;           // keeps first, last, extremes per bucket
pub fn reset_marks(sorted_resets: &[UnixSeconds], kind: &WindowKind) -> Vec<UnixSeconds>;   // D4.5, at most 64
pub fn gap_for(bucket: Duration) -> Duration;                                        // max(75 min, 2.5 x bucket)
pub fn range_bounds(range: HistoryRange, now: UnixSeconds) -> (UnixSeconds, UnixSeconds);
pub const RAW_RETENTION: Duration; pub const ROLLUP_RETENTION: Duration;              // 45 d, 400 d
// settings
impl Settings { pub fn defaults() -> Self; pub fn validated(self) -> Self; pub fn apply(&self, p: SettingsPatch) -> Result<Self, CommandError>; pub fn thresholds(&self) -> Thresholds; }
// text (D9.9, D9.11, D9.12) — the only place tray/notification strings are built
pub fn percent(p: Percent) -> String;
pub fn window_name(k: &WindowKind) -> String;
pub fn reset_absolute(at: UnixSeconds, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String;
pub fn countdown(at: UnixSeconds, now: UnixSeconds) -> Option<String>;               // None when past
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct TrayRow { pub provider: ProviderKey, pub text: String }                   // a row opens its provider (D9.11)
pub fn tray_reset(at: UnixSeconds, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String;   // "14:00" | "Thu 02:00" | "16 Dec"
pub fn stale_stamp(at: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String;                    // "16 Dec 12:31": always date and time
pub fn tray_rows(snapshot: &Snapshot, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> Vec<TrayRow>;   // D9.11: reading, stale, elapsed and no-reading rows
pub fn notification(alert: &Alert, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> (String, String); // title, body (absolute time only, D9.12)
// view — domain to IPC read model
pub struct ProviderState<'a> { pub provider: ProviderKey, pub usage: &'a Usage, pub refresh: RefreshView }
pub fn snapshot(states: &[ProviderState<'_>], now: UnixMillis, settings: &Settings) -> Snapshot;
pub fn same_content(a: &Snapshot, b: &Snapshot) -> bool;                              // ignores generated_at
```

**Acceptance tests.**
- `derive`: table test over every combination of (`attempted`, `last_good` age, failure
  presence/kind) → the exact `Usage` of D4.3/D7.7; a `NotConfigured` or `Unsupported` error
  never displaces a reading; property: total (never panics), never
  returns `Known` with an old reading, never loses `last_good` on failure; a reading whose
  `observed_at` is after `now` is treated as age zero (no underflow).
- `alerts::step` properties over arbitrary reading sequences: at most one alert per
  `(meter, level)` between re-arms; critical suppresses warn in the same step; no alert for
  stale readings, elapsed windows, disabled levels, the master switch off, hidden providers
  or balance meters; a crossing while notifications are off is still **recorded**, so
  switching them on afterwards fires nothing; restart safety
  (`step` on a memory round-tripped through the store type gives no duplicates); hysteresis
  of 5 points; re-arm on passed reset.
- `headline`: picks the highest non-elapsed window; ignores hidden providers, `Unsupported`,
  `Pending`; deterministic tie-break by `ProviderKey` order.
- `text` and formatting: every case in `dashboard/fixtures/format-vectors.json` (the same
  file the UI tests use); zone and hour-cycle variants; DST transition days; year rollover.
  `tray_reset` never produces "today" or "tomorrow"; a stale reading's tray row ends with
  "as of" plus date and time; an elapsed window's row has no number; a provider with no
  reading has exactly one row for every `Unavailable` and `Setup` value and for pending;
  hidden and unsupported providers have none; the notification body has no relative time
  and is empty when the alert has no reset time.
- `reset_marks`: a fixed `resets_at` gives one mark per window; a `resets_at` that slides by
  seconds on every sample still gives one mark per window length; at most 64. `gap_for`
  values for each range's bucket.
- `severity`: a window by its `used` against the thresholds, `Unknown` when elapsed; a
  balance `Critical` when not usable, else `Ok`.
- `should_store`/`downsample`: bounds (`len ≤ max`), first and last kept, extremes kept,
  monotone timestamps; `range_bounds` for each range.
- `Settings`: defaults (80/95; notifications master on and both levels on; system theme;
  auto hour cycle; `zoom_pct` 100; close to tray on; **autostart on** (D1a); shortcuts on; live check off;
  `first_run_seen` false; no hidden provider); `validated` clamps to 50..=100, enforces
  warn < critical and snaps `zoom_pct` to 100/125/150/175/200;
  `apply` rejects invalid patches with `InvalidArgument`; JSON round-trip; unknown fields in
  a stored file are ignored; a wrong `version` yields defaults.
- `view::snapshot`: canary cannot appear (inputs are typed); JSON matches the bindings
  (snapshot test of one full example).

**Security rules.** D8.4 (strings built only from typed inputs), D7.4 (no new field).

**Depends on.** T00.

**Review checklist.**
- [ ] No clock, IO, randomness or global state anywhere in these files.
- [ ] `alerts::step` invariants are property-tested, not only example-tested.
- [ ] `text` output for every vector equals the UI formatter's output (shared file).
- [ ] No arithmetic that can overflow or underflow (saturating/checked everywhere).

---

## T03 — Claude gateway log source: tail, watch, backfill (wave 1)

**Goal.** Event-driven, bounded-memory reading of `usage.jsonl` that survives rotation and
can only ever hand lines to T01's parser.

**Owns.** `dashboard/crates/usage-io/src/{tail.rs,watch.rs,sources/claude_log.rs}`,
`dashboard/crates/usage-io/tests/claude_log_*.rs`.

**Exposes.**
```rust
// tail
pub struct TailReader { /* path, (dev, inode), offset, carry buffer (zeroized on drop) */ }
pub enum TailEvent { Unchanged, Read { lines: u64 }, Rotated { lines: u64 }, Missing }
impl TailReader {
    pub fn new(path: PathBuf) -> Self;
    /// Cold start (D6.1). Scans backwards from EOF in blocks (≤ max_bytes), calling `accept`
    /// for every complete line of each block read, and stops after the first block in which
    /// `accept` returned true at least once. Falls back to `<path>.1`. Leaves the reader
    /// positioned at EOF of `<path>`. Returns whether any line was accepted.
    pub fn seek_newest(&mut self, max_bytes: u64, accept: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, Unavailable>;
    /// Feed every complete new line since the last call, following a rotation.
    pub fn read_new(&mut self, on_line: &mut dyn FnMut(&[u8])) -> Result<TailEvent, Unavailable>;
}
/// Whole-file forward scan for the backfill; same line rules, constant memory.
pub fn for_each_line(path: &Path, on_line: &mut dyn FnMut(&[u8])) -> Result<u64, Unavailable>;
pub const BLOCK: usize = 256 * 1024; pub const MAX_LINE: usize = 1024 * 1024;
// watch
pub struct WatchGuard;   // dropping it stops the watch and its re-arm timer
/// inotify (`inotify` crate, its descriptor in tokio's `AsyncFd` on `rt`'s reactor, no thread
/// and no stream adapter, D8.9): MODIFY on the file, and
/// CREATE | MOVED_TO | MOVED_FROM | DELETE on the parent directory filtered to the file name.
/// Throttled: the first event arms `coalesce`, the signal is sent when it fires (D7.2).
/// Never fails: when the directory is missing or the watch is lost it re-arms itself at
/// 1, 2, 4 … 15 min and signals once when armed. A queue overflow is a signal.
pub struct WatchOptions { pub coalesce: Duration, pub rearm_first: Duration, pub rearm_max: Duration }   // Default: 1.5 s, 1 min, 15 min
pub fn watch_file(path: &Path, id: SourceId, tx: mpsc::Sender<SourceId>, opts: WatchOptions,
                  rt: &tokio::runtime::Handle) -> WatchGuard;
// sources::claude_log
pub struct ClaudeGatewayLogSource;
/// `path` None (no home, no state directory) answers `NotConfigured(UseGateway)`. The clock
/// decides when a carried window has expired (D6.1).
impl ClaudeGatewayLogSource { pub fn new(path: Option<PathBuf>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self; }
impl UsageSource for ClaudeGatewayLogSource   // Trigger::OnSignal, Cost::Free, timeout 5 s, runs IO in spawn_blocking
/// First-run history backfill: `<path>.1` then `<path>`, in batches of 512 readings through
/// `HistoryStore::record_all` (which applies the write rule). Returns samples stored.
pub fn backfill(path: &Path, store: &dyn HistoryStore) -> Result<u64, StoreError>;
```
The source holds the per-window state of D6.1 (latest value and `ts_ms` per window name,
behind a mutex): rows are **not** in `ts_ms` order, a row replaces a window only when its
`ts_ms` is greater, and a window missing from the newest row is carried while its reset is
in the future and it is younger than 7 days. `Reading.observed_at` is the greatest `ts_ms`
used; `plan` is `facts.get().plan(Claude)`. A fetch that finds no newer sample returns the
reading it already holds. No sample anywhere → `Err(NotConfigured(UseGateway))`. File
missing → the same. Permission or read error → `Err(Unavailable(Io))`.

**Acceptance tests** (temp dirs, synthetic rows from `fixtures/claude-gateway/`).
- Property: for any split of a file into appended chunks (including splits inside a line and
  inside a multi-byte character) and any rotation point (rename to `.1` + new file, with or
  without unread bytes in the old one), the sequence of lines delivered equals a whole-file
  read; no line twice, none lost, none partial.
- Truncation (size below offset) restarts at 0. A line longer than `MAX_LINE` is skipped and
  the next line is delivered. Memory stays below `BLOCK + MAX_LINE` (assert on buffer capacity).
- `seek_newest` finds the newest sample when the last N rows have no `ratelimit`; honours
  `max_bytes`; falls back to `.1`; feeds every line of the blocks it read.
- **Out-of-order rows**: a file whose last line has an older `ts_ms` than the line before it
  yields the reading of the greater `ts_ms`; property: for any permutation of a set of rows,
  the source's final reading is the same.
- **Sporadic window**: rows `[5h,7d,7d_oi]`, `[5h,7d]`, `[5h,7d]` yield three meters after
  each fetch; the carried `7d_oi` disappears once its reset has passed (fake clock) or after
  7 days.
- A signal with no new sample rows returns the held reading, not an error.
- `watch_file`: 200 appends in 100 ms produce one signal; appends every 200 ms for 5 s
  produce a signal about every `coalesce` (no starvation); a rename-rotation produces a
  signal and later appends to the **new** file still signal; delete-and-recreate produces a
  signal; writes to a sibling file produce none **and wake nothing** (an event counter in
  `test-support` stays at zero); creating or deleting a sibling wakes the task once and
  produces no signal (the directory watch reports it, the name filter drops it); no
  signal with no writes for 2 s; a watch on a directory that does not exist yet arms itself
  after the directory appears (short `WatchOptions` re-arm times) and signals once; dropping the
  guard stops signals. (Real inotify in a temp dir, bounded waits.)
- Source: fixture file → expected `Reading`; `canary.jsonl` → no canary in the result;
  the source never opens any path other than `<path>` and `<path>.1` (test with a sibling
  `gateway.jsonl` whose permissions are 000: the source still succeeds).
- `backfill` on a two-file fixture with out-of-order rows stores the expected number of
  samples (older-than-last rows dropped) and a second run stores nothing.

**Security rules.** D8.3 (only the two paths; read-only open; buffers private and zeroized
on drop; lines go only to `parse::claude_log`), D6.1 (never create/truncate/lock/write the
log), D8.4 (no line content in errors or `diag`).

**Depends on.** T00 (uses T01's `parse_line` and `might_have_sample`).

**Review checklist.**
- [ ] Files opened with read-only flags only; no `create`, no locks.
- [ ] No polling loop, no `sleep` in a loop, no watcher thread; only the two narrow inotify
      watches and the throttle timer.
- [ ] Rotation logic proven by the property test, including the inode check.
- [ ] No `String::from_utf8_lossy`/logging of line bytes anywhere.
- [ ] The only file names in this code are derived from the injected path.

---

## T04 — Subprocess: the spawner, the login probe, Codex app-server, Claude live check (wave 1)

**Goal.** The single, locked-down process spawner, the narrow login probe and the two
sources that use them.

**Owns.** `dashboard/crates/usage-io/src/{proc.rs,probe.rs,sources/codex.rs,sources/claude_live.rs}`,
`dashboard/crates/usage-io/tests/{proc_*.rs,probe_*.rs,codex_*.rs,claude_live_*.rs}`, test stub
programs under `dashboard/crates/usage-io/tests/stubs/` (shell scripts).

**Exposes.**
```rust
// proc (the only module allowed to use std/tokio process APIs)
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum Program { Codex, Claude }
pub enum Stderr { Discard, Merge }       // Merge (2>&1) only for the two login-status specs
pub struct ProcSpec { pub program: Program, pub args: &'static [&'static str],
                      pub stdin: &'static [u8], pub deadline: Duration,
                      pub stderr: Stderr, pub max_output: usize }
pub enum Flow { Continue, Done }      // Done: close stdin and wait for exit
pub struct Exit { pub success: bool }
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum Resolved { Installed, NotInstalled }
/// D8.6 resolution, pure over injected directory reads: first PATH hit; a mise launcher
/// counts only when a version directory under `<mise data>/installs/<tool>/` holds the
/// executable `<tool>` or `bin/<tool>`.
pub fn resolve_in(name: &str, path_dirs: &[PathBuf], mise_data: Option<&Path>) -> Resolved;
pub struct ProcRunner;
impl ProcRunner {
    pub fn system(workdir: PathBuf) -> Self;                                   // "codex"/"claude" by name, through PATH
    pub fn with_programs(workdir: PathBuf, codex: PathBuf, claude: PathBuf) -> Self;   // feature "test-support"
    pub fn resolve(&self, program: Program) -> Resolved;                       // runs nothing
    /// Spawn under the rules of D8.6, write `stdin`, call `on_line` per output line.
    pub async fn run(&self, spec: &ProcSpec, on_line: &mut (dyn FnMut(&[u8]) -> Flow + Send)) -> Result<Exit, Unavailable>;
}
pub const ENV_ALLOW: &[&str];            // D8.6 list of inherited names
pub const ENV_CONST: &[(&str, &str)];    // TERM=dumb, DISABLE_AUTOUPDATER=1 and the four MISE_* switches of D8.6
// probe — body of `CliProbe` (types are T00's): D8.6 steps 1–4, output into a zeroizing buffer
// sources::codex
pub struct CodexAppServerSource;
impl CodexAppServerSource { pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self; }
impl UsageSource   // Interval { visible: 5 min, hidden: 15 min }, Cost::Free, timeout 15 s
// sources::claude_live
pub struct ClaudeLiveSource;
impl ClaudeLiveSource { pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self; }
impl UsageSource   // Trigger::Manual, Cost::SpendsQuota, timeout 60 s
```
Codex request bytes are a `const` equal to lines 1–3 of
`fixtures/codex-app-server/requests.jsonl` (no `account/usage/read`, no `account/read`).
Both sources set `needs_login: true` in their descriptor.
Readiness gate: `facts.get().readiness(p)` must be `Ready`, else return
`Unavailable(NotInstalled | NotLoggedIn)`, or `Unavailable(Timeout)` for readiness `Unknown`,
**without spawning** (D6.3). Error mapping: spawn failure → `NotInstalled`; deadline →
`Timeout`; non-zero exit with no response → `Protocol`; output over the caps → `Protocol`.
The child leads its own process group; the group is killed on the deadline **and when the
`run` future is dropped** (a guard, disarmed after the child is reaped), D8.6.
Claude live flags: determine from `claude --help` only (do not run a prompt). The source is
unit-tested with a stub program that replays `tests/fixtures/claude/turn1.jsonl`. A real run
is not part of this task.

**Acceptance tests** (stub programs only).
- Stub replays the recorded Codex session (with notifications interleaved and a 200 ms
  delay) → the expected `Reading`; stdin received by the stub equals the three request
  lines exactly.
- Failure matrix: stub exits 1 immediately; prints garbage; prints a 5 MiB line; never
  answers (deadline with paused time, process group is killed: assert no child remains);
  answers with a JSON-RPC error (`ServerError`); closes stdout early.
- **Launcher shape**: the stub is a wrapper that starts a second process which does the
  work (as the mise wrappers do). On the deadline, and when the `run` future is dropped
  half-way, neither process remains.
- **Environment**: parent has `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`, `JEV_API_KEY`,
  `ANTHROPIC_BASE_URL`, `HTTPS_PROXY`, `GITHUB_TOKEN` set to canary values; the stub dumps
  its environment; assert only `ENV_ALLOW` and `ENV_CONST` names are present, the four
  `MISE_*` switches have their D8.6 values, and no canary. Both lists equal D8.6 exactly
  (snapshot test).
- **Resolution** (temp dirs, nothing run): a plain executable → `Installed`; no hit →
  `NotInstalled`; a hit under `…/command-wrappers/bin/`, under `…/shims/`, or a symlink to a
  file named `mise` → `Installed` only when `installs/<tool>/<version>/<tool>` or
  `…/<version>/bin/<tool>` is an executable regular file, and `NotInstalled` with no
  version directory, an empty one, one that holds only other files (an interrupted
  download) or a non-executable file of that name; a stub wrapper that behaves like the
  account wrapper when the selected version is missing (it calls a stub `mise install`)
  shows the D8.6 limit: the stub `mise` sees `MISE_OFFLINE=1` and nothing is written; the mise data directory
  follows `MISE_DATA_DIR`, then `XDG_DATA_HOME`, then the home. With `NotInstalled` the
  stub's marker file is never written by the probe or by either source.
- **Login probe** (stubs): `codex` stub that prints `Logged in using ChatGPT` on **stderr**
  and exits 0 → `Ready`; prints `Not logged in` or exits 1 → `NotLoggedIn`; hangs →
  `Unknown` after 5 s (paused time) with the group killed; `claude` stub replaying
  `fixtures/claude-auth/*` → `Ready` with plan `max_5x`, `NotLoggedIn`, and `Unknown` for
  garbage; a hidden provider is not probed (marker absent); the canary e-mail of the fixture
  is absent from `LoginFacts` Debug and from `diag` capture; the probe's buffer is zeroized
  (wrapper type that records it).
- Readiness `Unknown` → `Unavailable(Timeout)` and no process started.
- stderr of the child containing the canary never reaches `diag` capture or the result.
  `proc`'s own read buffers are zeroized when a run ends (same recording wrapper as the
  probe's buffer).
- Working directory of the child is the given workdir. No shell is involved (`ps` of the
  stub shows the program itself as argv0).
- Readiness `NotLoggedIn`/`NotInstalled` → no process started (stub writes a marker file
  when run; assert absent).
- There is no way to construct a `Program` for any other binary (compile-level: the enum).
- **Manual check, recorded in the report** (free, no prompt, allowed by D3.1): through
  `ProcRunner::system` with the scrubbed environment, the `MISE_*` switches and the empty
  working directory, on this machine: both login-status commands give `Ready` with the
  right plan, and one real `codex app-server` run returns a reading with the three request
  lines written at once (D6.3; if the server needs them one by one, report it: the fix
  changes the frozen `proc` types and is the lead's decision). List mise's
  `installs/`, `downloads/` and `incomplete-installs/` directories and hash
  `~/.config/mise/config.toml` before and after: no
  entry may appear or change and the hash must be equal (the D8.6 assumptions that nothing
  is downloaded, by mise or by a CLI's own updater, and that the account wrapper's
  `mise use -g` fallback is not reached). If a run
  fails, report which variable was missing or which switch mise refused; do not widen
  `ENV_ALLOW` or drop a switch without the lead.

**Security rules.** D8.6 in full, D8.3 (`account/read` never sent: test greps the request
constant), D6.2 (live check only on manual trigger), D8.4.

**Depends on.** T00 (uses T01's `parse::codex` and `parse::claude_event`).

**Review checklist.**
- [ ] `env_clear()` then allow-list; own process group; group kill on deadline and on drop.
- [ ] stderr is `Stdio::null()` except for the two status specs (merged); output bounded per
      line and in total.
- [ ] Nothing is started for a program that `resolve` calls `NotInstalled`; the probe opens
      no CLI home file and holds the status output only in a zeroizing buffer.
- [ ] No `sh -c`, no formatted argument strings; args are `&'static`.
- [ ] The Codex request constant contains exactly `initialize`, `initialized`,
      `account/rateLimits/read`.
- [ ] `ClaudeLiveSource` cannot run from a timer (its trigger is `Manual`).

---

## T05 — DeepSeek balance: the one HTTP client and the one secret (wave 1)

**Goal.** Fetch the DeepSeek balance with a key that can never go anywhere else.

**Owns.** `dashboard/crates/usage-io/src/{http.rs,secrets.rs,sources/deepseek.rs}`,
`dashboard/crates/usage-io/tests/deepseek_*.rs`.

**Exposes.**
```rust
// http (the only module allowed to use reqwest)
#[derive(Copy, Clone, Debug)] pub enum Endpoint { DeepSeekBalance }
pub struct HttpClient;
impl HttpClient {
    pub fn new() -> Result<Self, Unavailable>;                         // D8.7 settings
    pub fn with_base(base: &str) -> Result<Self, Unavailable>;         // feature "test-support": local stub server (http allowed only here)
    pub async fn get_json(&self, endpoint: Endpoint, bearer: &Secret<String>) -> Result<Vec<u8>, Unavailable>;
}
// secrets
pub struct KeySources { pub env: Option<Secret<String>>, pub env_file: Option<PathBuf> }
impl KeySources { pub fn system() -> Self; }                           // process env + engine_bridge::env_file()
/// Environment first, else the env file through `io::envfile::read_value` (T00; zeroizing, the engine's parse rule).
/// By value: `Secret` is not `Clone`, so the environment's key is moved out, never copied.
pub fn deepseek_key(sources: KeySources) -> Option<Secret<String>>;
// sources::deepseek
pub struct DeepSeekBalanceSource;
impl DeepSeekBalanceSource { pub fn new(http: Arc<HttpClient>, clock: Arc<dyn Clock>, keys: Arc<dyn Fn() -> KeySources + Send + Sync>) -> Self; }
impl UsageSource   // Interval { visible: 10 min, hidden: 30 min }, Cost::Free, timeout 10 s
```
Status mapping: 200 → parse; 401/403 → `Unauthorized`; 429 → `RateLimited`; 5xx →
`ServerError`; connect/DNS failure → `Offline`; timeout → `Timeout`; body over 64 KiB or
unparsable → `Protocol`. No key → `Err(NotConfigured(AddDeepSeekKey))` and **no request**.

**Acceptance tests** (local stub server on 127.0.0.1, ephemeral port).
- Happy path with `fixtures/deepseek/balance_ok.json` → expected balance meter; two
  currencies → two meters.
- The request the stub receives: method GET, path `/user/balance`, `Authorization: Bearer
  <key>`, `User-Agent: jev-dashboard`, no cookies, no other custom headers.
- Each status/failure mapping above; a redirect response is **not followed** (stub counts
  hits on the redirect target: zero); a slow body hits the timeout; a 1 MiB body is cut off.
- **Key canary**: key = `JEVCANARY_KEY_…`. After success and after every failure: the
  canary is absent from the returned value's Debug, from `diag` capture, and from the Debug
  of every error; `expose_secret` is called in exactly one place, `http.rs` (source grep in
  the test).
- `deepseek_key`: env wins over file; file with `export`, quotes, comments; missing file;
  empty value → `None`; the value moves from `envfile::read_value` into the `Secret`
  without a second copy.
- No key → the stub server receives no connection.
- `HttpClient::new()` builder settings asserted through a unit test of the config function
  (`https_only`, no redirects, no proxy, timeouts).

**Security rules.** D8.5 (key lifetime = one fetch; never cloned, logged, stored, sent to
IPC; header marked sensitive), D8.7 (one client, one host, HTTPS only, no redirect, no
proxy, body cap), D8.4 (reqwest errors classified and dropped; never `to_string()`ed).

**Depends on.** T00 (uses T01's `parse::deepseek`).

**Review checklist.**
- [ ] `Secret` never leaves the fetch call's stack; no field of a long-lived struct holds it.
- [ ] `reqwest::Error` is matched by kind (`is_timeout`, `is_connect`, status) and discarded.
- [ ] The production URL is a `const` with `https://api.deepseek.com/user/balance`; the test
      base override is behind `test-support` only.
- [ ] `HeaderValue::set_sensitive(true)` on the authorization header.
- [ ] No environment proxy, no cookie store, no automatic decompression features enabled.

---

## T06 — Persistence: SQLite history, settings file, private files (wave 1)

**Goal.** Durable, private, migratable storage of numbers only.

**Owns.** `dashboard/crates/usage-io/src/{fsutil.rs,store.rs,settings_file.rs}`,
`dashboard/crates/usage-io/tests/{store_*.rs,settings_*.rs,fsutil_*.rs}`.

**Exposes.**
```rust
// fsutil
// `FsError` is T00's field-less enum; `std::io::Error` is classified here and dropped (D8.4)
pub fn ensure_private_dir(path: &Path) -> Result<(), FsError>;                 // create 0700, tighten if looser
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), FsError>; // tmp 0600 + fsync + rename + dir fsync
pub fn remove_file(path: &Path) -> Result<(), FsError>;                        // a missing file is Ok
// store
pub struct SqliteStore;
impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError>;       // pragmas, migrate, corrupt → rename aside + recreate
    pub fn meta_get(&self, key: MetaKey) -> Result<Option<i64>, StoreError>;
    pub fn meta_set(&self, key: MetaKey, value: i64) -> Result<(), StoreError>;
}
// MetaKey is defined by T00: BackfillDone, LastPruneAt, WindowWidth, WindowHeight
impl HistoryStore for SqliteStore   // record/record_all apply history::should_store; series aggregates in SQL (D7.5); stats
/// feature "test-support": rows stepped from SQLite by the last `series` call.
impl SqliteStore { pub fn last_series_rows(&self) -> u64; }
pub const SCHEMA_VERSION: i64 = 1;
// settings_file
pub struct JsonSettingsStore;
impl JsonSettingsStore { pub fn new(path: PathBuf) -> Self; }
impl SettingsStore for JsonSettingsStore   // load: missing/invalid → defaults (invalid file renamed *.invalid); save: write_private_atomic
```
Schema, pragmas, retention and migration rules: D7.5. `series` returns `SeriesData`: points
with `lo/hi` (equal to `value` for raw samples), never more than `max_points`, aggregated by
SQL `GROUP BY` so the raw rows of a range are never loaded; the bucket width; and the reset
marks, read as at most 130 grouped rows and collapsed by `history::reset_marks`, empty for
a rollup query and when more than 64 would result (D7.5). The store is given only `from`
and `to`: a span up to `RAW_RETENTION` (day, week, month) comes from `sample`; a longer one
(quarter) is a rollup query, from `rollup_hour` plus `sample`. `stats` is the trait method (`StoreStats`
is a core type).
The database file is created 0600 by the store before SQLite opens it (D7.5). All text written to the
database comes from `ProviderKey::as_str()` and `MeterId::key()`; all SQL uses bound
parameters.

**Acceptance tests** (temp dirs).
- Modes: directory 0700; `history.db`, `-wal`, `-shm`, `settings.json` and the atomic temp
  file 0600, also when the process umask is 022 at test time.
- `record` + `series` round-trip; write rule (unchanged value within 15 min not stored;
  changed value stored; heartbeat stored; a reading older than or in the same second as the
  last stored one is dropped, not an error); `record_all` of 10 000 readings is one
  transaction and equals 10 000 `record` calls in effect.
- Memory: `series` over 100 000 raw samples whose `resets_at` slides on every sample returns
  ≤ `max_points` points and `last_series_rows()` ≤ `max_points + 130`.
- Reset marks: returned for day, week and month; empty for quarter; empty when more than 64
  would result.
- Retention with a fake `now`: raw older than 45 d gone, rolled into hours first; rollups
  older than 400 d gone; `series` over a quarter merges rollups and raw without overlap.
- Property: for arbitrary sample sequences, `series(..., max)` has `len ≤ max`, sorted,
  within range, and contains the global min and max of the range.
- Migration: a v0 (empty) file migrates to v1; a file with `user_version = 99` is not
  written (returns `Unavailable` on `record`, still serves `series`); a file of random bytes
  is renamed to `history.db.corrupt-*` and a fresh store works.
- Crash safety: kill a child process mid-transaction (the test re-executes its own test
  binary with a marker variable; no extra build target), reopen, database
  is consistent (`PRAGMA integrity_check`).
- Alert memory round-trip. Meta round-trip.
- Settings: round-trip; invalid JSON → defaults and `settings.json.invalid` exists;
  unknown fields ignored; atomic write leaves no temp file; concurrent `save` calls end
  with one valid file.
- **Canary**: there is no API that accepts a free string; a test asserts that the SQLite
  file bytes after a full write cycle contain only expected text (table names, provider and
  meter keys).

**Security rules.** D8.4 (numbers and closed keys only), D7.5 (0600/0700, secure_delete),
no SQL string concatenation.

**Depends on.** T00 (uses T02's `history::should_store`, `downsample`, retention constants).

**Review checklist.**
- [ ] Every statement uses bound parameters; no `format!` into SQL.
- [ ] All writes in transactions; prune + rollup atomic.
- [ ] One connection, `Mutex`, callers use `spawn_blocking` (documented on the type).
- [ ] Permissions asserted by tests, not assumed from the umask.
- [ ] A newer or corrupt database can never crash the app or lose the user's settings.

---

## T07 — UI foundation: tokens, formatting, IPC seam, stores, app shell (wave 1, UI-design pass)

**Skills.** Load `frontend-design` for the visual direction within D9.1/D9.7, then
`impeccable` for the system. (The primitives were split out into T14, which runs beside
this task.) Tokens and wireframes in D9 are the contract;
propose deviations to the lead, do not just apply them.

**Goal.** Tokens, formatting, the IPC seam, stores, router, app shell and the Playwright
harness, so the primitives and screen tasks only compose.

**Owns.** `dashboard/src/lib/design/**` (tokens.css, base.css, fonts),
`dashboard/src/lib/format/**`, `dashboard/src/lib/copy.ts`,
`dashboard/src/lib/ipc/{tauri.ts,mock.ts,fixtures.ts}` (`client.ts` is T00's and frozen),
`dashboard/src/lib/stores/**` (rune files are `*.svelte.ts`: `usage.svelte.ts`,
`settings.svelte.ts`, `clock.svelte.ts`, `announce.svelte.ts`),
`dashboard/src/lib/router.svelte.ts`, `dashboard/src/lib/boot.ts` (sets the theme; the
first import of the entry module, D9.7), `dashboard/src/lib/components/shell/**` (the one
header and the one live region; T14 has neither), `dashboard/src/App.svelte`, `dashboard/src/main.ts`, `dashboard/index.html`,
their unit/component tests, `dashboard/e2e/screens/_harness/**` (Playwright helpers: theme/
width matrix, axe runner, no-network guard, CSP-violation guard) and
`dashboard/e2e/screens/shell.spec.ts` with its baselines.

**Exposes.**
```ts
// lib/ipc/client.ts (T00's, frozen) — components and stores import ONLY this, never @tauri-apps/api.
// `Outcome<T>` is the generated envelope of D7.3; nothing here is hand-typed twice.
export interface IpcClient {
  getSnapshot(): Promise<Outcome<Snapshot>>;
  refresh(target: RefreshTarget): Promise<Outcome<null>>;
  getHistory(query: HistoryQuery): Promise<Outcome<HistorySeries>>;
  getSettings(): Promise<Outcome<Settings>>;
  updateSettings(patch: SettingsPatch): Promise<Outcome<Settings>>;
  getDiagnostics(): Promise<Outcome<Diagnostics>>;
  onSnapshot(cb: (s: Snapshot) => void): Promise<() => void>;
  onNavigate(cb: (r: Route) => void): Promise<() => void>;
}
// lib/ipc/tauri.ts: `invoke<Outcome<T>>` with generated type arguments; the rejection handler
// takes NO parameter and returns the constant invalid_argument outcome (D7.3).
export function createTauriClient(): IpcClient;
// lib/ipc/mock.ts and fixtures.ts: every name is a declared type (D1a)
export type FixtureName = "pending" | "known" | "stale_quiet" | "stale_failing" | "elapsed" | "unavailable"
                        | "not_configured" | "first_run" | "mixed";    // one per state of D9.5 + mixed
export type MockScript = { snapshot: Snapshot; settings: Settings; history: HistorySeries; diagnostics: Diagnostics;
                           fail: Partial<Record<"refresh" | "getHistory" | "updateSettings", CommandError>> };
export interface MockIpcClient extends IpcClient { push(s: Snapshot): void; navigate(r: Route): void;
                                                   readonly calls: readonly MockCall[] }
export type MockCall = { command: "getSnapshot" | "refresh" | "getHistory" | "getSettings" | "updateSettings" | "getDiagnostics" };
export function createMockClient(script: MockScript): MockIpcClient;
export const fixtures: Record<FixtureName, MockScript>;   // a whole script: `first_run` differs in `settings`, not in the snapshot

// lib/stores
export const usage:    { readonly snapshot: Snapshot | null; readonly refreshing: boolean; refresh(t?: RefreshTarget): Promise<Outcome<null>> };
export const settings: { readonly value: Settings; update(p: SettingsPatch): Promise<Outcome<Settings>> };
export const clock:    { readonly now: number };                 // epoch ms; ticks on the minute, only while visible
export const router:   { readonly route: Route; go(r: Route): void; back(): void };   // initial route and theme from the URL fragment (D7.3)
/// The single polite live region's queue (D9.10): merges messages within 2 s, drops a repeat within 60 s.
export const announce: { say(message: string): void; readonly current: string };
/// Subscribes to `snapshot` BEFORE calling `getSnapshot`, keeps the greater `generated_at` (D7.3).
export function initStores(client: IpcClient): Promise<() => void>;

// lib/format (pure; D9.9; tested against fixtures/format-vectors.json)
export function percent(bp: number): string;
export function countdown(resetsAtS: number, nowMs: number): string | null;
export function absolute(resetsAtS: number, nowMs: number, hour: HourCycle, zone?: string): string;
export function age(atMs: number, nowMs: number, hour: HourCycle): string;
export function money(micros: number, currency: Currency): string;
export function windowName(w: WindowKind): string;
export function meterValueText(m: MeterView, nowMs: number, hour: HourCycle): string;   // aria-valuetext

// lib/copy.ts — every user-facing sentence for Unavailable, Setup, StaleCause; commands to copy
export function unavailableCopy(reason: Unavailable, provider: ProviderKey): { text: string; command?: string };
export function setupCopy(setup: Setup, provider: ProviderKey): { text: string; command?: string };
export const providerName: Record<ProviderKey, string>;
export function planName(label: string): string;   // "max_5x" → "Max 5x", "free" → "Free"; unknown → "_" as spaces, capitalised
export function levelChange(provider: ProviderKey, meter: MeterView, nowMs: number, hour: HourCycle): string;   // live-region sentence, same words as the notification
```
`App.svelte` mounts the header, the router outlet with the three screen files (owned by
T09/T10), the single live region (fed by `announce`), global shortcuts (off when
`settings.shortcuts` is false or a text field has focus; `1`–`6` open the providers in
`ProviderKey::ALL` order, a hidden one does nothing; `?` goes to Settings, D9.10), the zoom keys (Ctrl + plus, minus,
zero call `settings.update({ zoom_pct })`, D9.10; they are not single-key shortcuts and stay
on) and the document title per view. The header (back control, view title, freshness,
refresh, settings) and the live region are T07's own components in
`lib/components/shell/`, built to the same rules as T14's primitives (32 px targets, text
names, tokens only), so nothing has to be swapped after T14 merges; the screens of T09/T10
render below the header and bring no header of their own. `Esc` follows the one order of
D9.1 (dialog, then disclosure, then back). `main.ts` suppresses the web
view's context menu in production builds (D8.8). The live region announces level changes by
comparing consecutive snapshots: a meter whose `severity` changed, or a provider whose state
changed, produces one sentence; a changed number alone produces none.

**Acceptance tests.**
- `format` passes every vector in `format-vectors.json`; property tests with fast-check are
  not required, the vectors are.
- Contrast test computed from `tokens.css` for both themes meets D9.10 (text ≥ 4.5,
  primary ≥ 7, large numerals ≥ 4.5, control boundaries, focus ring, bar fill on track,
  status glyphs and chart lines ≥ 3), for every text colour on each of `bg`, `surface` and
  `surface-2`; every token is inside the sRGB gamut (D9.7).
- Palette test: the token file defines the neutrals, one accent, warn, critical and unknown
  and nothing else; `ok` is an alias of `accent` (D9.7).
- App shell (`shell.spec.ts`): header, empty outlet and live region in both themes at the
  three widths; zero axe violations; the document title follows the route; `lang="en"`.
- Zoom keys: Ctrl + plus steps 100 → 125 … 200 and stops; Ctrl + zero returns to 100; each
  sends one `updateSettings`.
- Shortcuts: `2` opens Codex whatever the card order is; the digit of a hidden provider
  does nothing; `?` and `,` open Settings; none acts in a text field or with
  `settings.shortcuts` off, while `Esc` and the zoom keys still do.
- `announce`/live region: a snapshot sequence in which a meter goes ok → warn announces one
  sentence; twenty snapshots that change only numbers announce nothing; three changes within
  2 s give one merged message; the same message is not repeated within 60 s; a background
  refresh does not announce "Updated".
- Stores: with the mock client, a pushed snapshot updates `usage`; a snapshot event that
  arrives before `getSnapshot` resolves is not overwritten by the older reply; `refreshing`
  follows `in_flight`; `Busy` is a no-op; `clock` stops ticking when `document.hidden`;
  router back/escape; the fragment `#view=settings&theme=dark` opens Settings with
  `data-theme="dark"` already set at `DOMContentLoaded`, while `<body>` is still empty
  (D9.7; the real web view's flash check is in T11's manual list); a fragment with unknown keys
  or values falls back to the overview and the system theme. `tauri.ts`: an `ok` and an `error` outcome pass through unchanged; a rejected `invoke`
  (Tauri's own argument error) becomes the `invalid_argument` outcome and its value is never
  bound to a name.
- Type discipline (D10): zero findings from `tsc`, `svelte-check`, ESLint and
  `check-types.ts` in everything this task owns, tests included; the `copy.ts` tables are
  `satisfies Record<Unavailable, …>` / `Record<Setup, …>` / `Record<ProviderKey, …>`, shown by
  a compile-fail note in the report (removing one entry breaks the build).
- Stylelint: no raw colour, font-size or spacing outside the token files. ESLint security
  rules pass (raw-HTML sinks, web storage, `style=` attributes). Bundle: fonts ≤ 120 KB.
- The harness serves the build with the CSP of `tauri.conf.json`; a `style=""` attribute or
  an inlined `data:` asset in a test page makes the CSP guard fail (shown once in the report).

**Security rules.** D8.8 (no inline styles as strings, no `{@html}`, no `innerHTML`; CSSOM
custom properties only), D8.7 (no runtime request: fonts and icons bundled), D7.4 (types
only from the generated bindings; no hand-written IPC types).

**Depends on.** T00.

**Review checklist.**
- [ ] Components import `IpcClient`, never `@tauri-apps/api`; no `try`/`catch` outside
      `ipc/tauri.ts` and the clipboard helper, and none with a binding.
- [ ] Tokens match D9.7 or the deviation was approved; no magic numbers in components.
- [ ] The live region is single, polite and not spammy (D9.10).
- [ ] Type tokens are in `rem`; one accent; a healthy palette is neutral plus one blue.
- [ ] Rune files are `*.svelte.ts`; nothing uses web storage; nothing sets a `style` attribute.
- [ ] No timer runs while the document is hidden.
- [ ] Fixtures cover every `UsageView` state, every `Unavailable` and `Setup` value.

---

## T08 — Usage service: the one loop (wave 2)

**Goal.** Wire sources, store, settings, alerts and the snapshot channel into the single
event-driven task of D7.1–D7.2.

**Owns.** `dashboard/crates/usage-io/src/{registry.rs,service.rs,timer.rs}`,
`dashboard/crates/usage-io/tests/{service_*.rs,timer_*.rs}`.

**Exposes.** Exactly D7.1 (`ServiceDeps`, `ServiceHandle`, `spawn(deps, rt)`), plus:
```rust
// registry
pub struct RegistryInputs { pub usage_log: Option<PathBuf>, pub facts: Arc<FactsCell>, pub clock: Arc<dyn Clock>,
                            pub runner: Arc<ProcRunner>, pub http: Arc<HttpClient>,
                            pub keys: Arc<dyn Fn() -> KeySources + Send + Sync> }   // production: KeySources::system
/// The one list of sources. Adding a provider is one line here. The live-check source is
/// always in the list; whether it may run is the setting, checked by the service at call time.
pub fn build(i: &RegistryInputs) -> Vec<Arc<dyn UsageSource>>;
/// The first-run backfill job, or None when `MetaKey::BackfillDone` is set or there is no log path.
/// The job runs `sources::claude_log::backfill` and then sets the meta key.
pub fn backfill_job(usage_log: Option<PathBuf>, store: Arc<SqliteStore>) -> Option<Backfill>;
// service
pub struct Backoff;   // pure: next_delay(failures, kind, jitter: f64) -> Duration   (D7.2)
pub struct SystemJitter;  impl Jitter for SystemJitter   // std RandomState, no `rand` (D8.9); `Jitter`, `Backfill`, `ShellFacts` are T00's types
// timer (D7.1): bodies of SystemTimer (timerfd, CLOCK_REALTIME, absolute, cancel-on-set) and FakeTimer
```
Behaviour to implement: D7.1 loop with **one** `WallTimer` wait (never
`tokio::time::sleep` for a deadline); D7.2 in full: signal sources, intervals, visibility,
backoff (`NotConfigured`/`Unsupported` are not failures), one-in-flight, the exact manual
refresh semantics (returns when queued; 10 s floor; `Busy`/`NotEnabled` cases; `live_check`
allowed only when the setting is on and the source's cost is `SpendsQuota`), and the engine
probe schedule (start; before a fetch of a `needs_login` source when not `Ready` and older
than 60 s, also while hidden; manual at most once per 60 s; piggy-back on a due fetch when
the facts are 6 hours old, or 15 minutes old with a probed provider still `Unknown`) through the
injected `LoginProbe`, called with the providers that are not hidden and have a
`needs_login` source, merging the result into `FactsCell` for the providers probed (the
others keep what they had, D7.1); freshness from each descriptor's
`fresh_for`; only `SourceError::Unavailable` is a failure, and a `Manual` source's failure
goes to `RefreshView.live_check_error` when the provider holds a reading (D4.3); merge of several sources per provider by
greatest `observed_at` (a strictly older reading is dropped, an equal one replaces; when a
probe changes a provider's facts its `OnSignal` sources are fetched once more, D4.4; a
future `observed_at` is clamped to now, D4.3); `assess::derive` per provider; `store.record` and `alerts::step` +
`notifier.notify(alert, settings.hour_cycle)` + `save_alerts` on each new reading, every store and settings call through
`spawn_blocking`; snapshot emitted only when `view::same_content` is false; the first-run
backfill (`ServiceDeps.backfill`) once, on a blocking thread, with `record` for that
provider held back (latest reading kept, recorded when the backfill ends) and alerts
suppressed until it finishes; daily prune; settings changes applied live (thresholds
re-evaluate severity and headline at once; hiding a provider cancels and stops its
fetches); `Wake::ClockChanged` recomputes every deadline; `shutdown()` cancels in-flight
fetches and ends the task within 2 s; dropping all handles does the same.

**Acceptance tests** (fake sources, `FakeClock`, tokio paused time, in-memory or temp store).
- Start: all `Pending`, then each provider settles as its fake source answers/fails.
- Signal → exactly one fetch of that source; 50 signals while a fetch is in flight → one
  more fetch after it; becoming visible fetches a signal source once.
- Interval source: fetched at the visible interval when visible, hidden interval when
  hidden; becoming visible with an old reading triggers one fetch; never two in flight.
- Backoff sequence 1, 2, 4 … 60 min (first failure → 1 min, D7.2) with injected jitter; reset on success; `RateLimited`
  starts at 15 min; `NotConfigured` never backs off; manual refresh bypasses it.
- Manual refresh: returns before the fetch completes; `all` twice within 10 s starts each
  source once; `provider` while its source is in flight → `Busy`; `all` never returns `Busy`.
- **Suspend**: the fake clock jumps forward 8 h past a window's reset and two interval
  deadlines; the fake timer fires at once; each due source runs once, not eight times, and
  the window is `elapsed` in the next snapshot.
- **Clock set**: `Wake::ClockChanged` with the clock moved back 1 h and forward 1 h: no
  panic, no negative age, deadlines recomputed, no fetch storm.
- A reading stamped 5 minutes in the future is stored and shown with `observed_at = now`.
- **Probe**: with a `ScriptedProbe` that first answers `Unknown` and then `Ready` for Codex,
  while hidden: the first gated fetch yields `Unavailable(Timeout)`, the retry re-probes and
  the source is fetched; the probe is called at most once per 60 s under 100 manual
  refreshes; a 6-hour-old `Ready` is re-probed on the next due fetch, with no extra wake;
  a Claude `Unknown` (its only `needs_login` source is manual) is re-probed on the first
  due fetch after 15 minutes, and not before.
- Failure after success → `Stale{Failing}` keeps the reading; quiet passive source →
  `Stale{Quiet}` after 10 min; window passing `resets_at` flips to elapsed and the headline
  changes **with no fetch** (timer-driven).
- Alerts: crossing 80 % notifies once; staying above does not repeat; restart with the same
  store does not repeat; critical only when both crossed; none when disabled; none for
  stale data; none during backfill. A `Notifier` whose `notify` would block for 10 s (a
  fake that records the call and parks a thread) does not delay the next snapshot.
- **Backfill order**: with a backfill job that takes 2 s and a live Claude reading arriving
  at 0.1 s, the snapshot shows the reading at once, the store receives the backfill rows
  first and the live sample after them, and no alert fires before the job ends. A failed
  job is retried at the next start (meta key not set) and does not block live recording.
- `live_check` refused when disabled; allowed when enabled; never scheduled by a timer; a
  failed live check on a provider with a reading leaves `usage` as it was and sets
  `live_check_error`, which the next successful check clears; `free_refresh` and
  `live_check` in `RefreshView` follow the registry and the setting.
- A hidden provider is not in the probe's provider list.
- Snapshot events: identical content → no event. **Idle proof**: with all sources quiet
  and no deadline within an hour, advancing the fake clock and paused time by 59 minutes
  runs zero task wakes (instrumented loop counter).
- `SystemTimer` (real timerfd, short waits): fires at a deadline 50 ms ahead; fires at once
  for a past deadline; a second `sleep_until` replaces the first.
- `shutdown()`: with a fetch in flight on a stub that never answers, returns within 2 s and
  the stub's process group is gone. `diagnostics()` returns store statistics without
  blocking the loop (a slow fake store delays only the diagnostics reply).
- History is recorded under the write rule; `history()` maps range → bounds → store.
- Liskov suite: the same behavioural tests run with each real adapter constructed over
  stubs/fixtures (uses `test-support` constructors).
- A panicking or hanging fake source cannot stall the loop (a hang → `Timeout`, a panic →
  `Unavailable(Internal)`, others continue). This holds in the shipped binary too, because
  `dist` keeps unwinding (D5); a manifest test asserts that no profile sets `panic`.
- A probe answer for Codex only leaves Claude's cached facts untouched.
- **Late plan**: the log source answers before the probe; when the probe then reports plan
  `max_5x`, the log source is fetched once more and the next snapshot carries the plan,
  with no new log row and no further fetch after that.

**Security rules.** D8.4 (`diag` only; no reading content logged), D6.2 (spending source
manual only), D7.4 (snapshot built only by `view::snapshot`).

**Depends on.** T01–T06.

**Review checklist.**
- [ ] One task owns all state; no `Mutex` around service state; no detached tasks that
      outlive the handle.
- [ ] Exactly one `WallTimer::sleep_until`; no tokio sleep for a deadline; no `interval`
      ticking when nothing is due.
- [ ] Nothing in the service is un-injected: clock, timer, jitter, probe, sources, stores,
      notifier, backfill all come from `ServiceDeps`.
- [ ] The service never matches on `SourceId`/`ProviderKey` to special-case behaviour: gating,
      freshness and cost all come from the descriptor.
- [ ] Every scheduling rule has a paused-time test; no real sleeps in tests.
- [ ] The idle proof test exists and is meaningful.

---

## T09 — Overview screen and all provider states (wave 2, UI-design pass)

**Skills.** `impeccable` while building; finish with a `better-ui` pass (radii, optical
alignment, hit areas).

**Goal.** The screen of D9.3 with every state of D9.5, composed from T14's primitives.

**Owns.** `dashboard/src/screens/Overview.svelte`, `dashboard/src/lib/components/overview/**`
(`ProviderCard`, `MeterRow`, `BalanceRow`, `CardState` variants for pending / stale-quiet /
stale-failing / elapsed / unavailable / not-configured, `FirstRun`, `UnsupportedLine`),
`dashboard/src/lib/overview/**` (pure helpers: `orderProviders`, `cardSummary`,
`rowDetail`), their tests, `dashboard/e2e/screens/overview.spec.ts` and its baselines.
There is no offline banner (D9.5).

**Consumes.** `usage`, `settings`, `clock`, `router`, `announce` stores; `format`, `copy`; T14's primitives.
No IPC calls except through the stores.

**Exposes.** `orderProviders(views: ProviderView[]): ProviderView[]` (D9.3 ordering, stable),
`cardSummary(view, nowMs, hour): string` (the card header button's accessible name; it
begins with the visible provider and plan text, D9.10 label in name),
`rowDetail(meter, observedAtMs, nowMs, hour): string` (the level-1 text, used both for the
`Detail` disclosure and inside `aria-valuetext`, so the two can never differ), the screen
component.

**Design contract (D9.1).** At rest a card shows level 0 only: name, plan, and per window
the number, the bar and one quiet line. No absolute time, status word or decimal at rest
unless a level is reached (then the level's glyph and word appear). Level 1 appears on hover
or focus of a row. Whitespace per D9.1. Nothing decorative.

**Acceptance tests.**
- A screenshot for every fixture state x light/dark x widths 360/720/1200, plus reduced
  motion and forced colors for the mixed fixture; baselines reviewed by the lead.
- Zero axe violations on every state x theme. Each card is a group with a heading; its
  header is one button named by `cardSummary`; every meter row is a tab stop (`role="meter"`,
  named, `tabindex="0"`, with the one approved `svelte-ignore` of D9.3; a row without a
  number is plain text, not a meter); inner controls
  are separate tab stops; no interactive element is nested in another; tab order equals
  reading order (asserted as a recorded focus sequence per fixture).
- **Progressive disclosure**: at rest the row shows no absolute time; focusing the row with
  Tab shows the level-1 detail, hovering shows the same text, `Esc` closes it and focus
  stays; the row's `aria-valuetext` contains the same detail with nothing opened; the
  detail never overlaps its row at 320 CSS px (both cases of D9.10).
- **Announcements**: the mixed fixture going ok → warn announces one sentence in the live
  region; twenty number-only updates announce nothing.
- Ordering unit tests (severity, then fixed order; no reorder on equal severity).
- Label in name (D9.10): for every button and meter row on every fixture, the computed
  accessible name contains the control's visible text in order.
- Type discipline (D10): zero findings in everything this task owns, tests and specs
  included; every component `lang="ts"` with typed props; state variants chosen by an
  exhaustive switch over `UsageView["state"]`.
- Pending skeleton appears only after 150 ms and stays ≥ 300 ms (fake timers).
- Elapsed window shows "—" and the reset sentence, never a number; stale-quiet keeps full
  contrast values and shows the age; stale-failing shows cause, saved time and Retry.
- Copy button copies the exact command and announces "Copied"; when the clipboard API
  rejects, the command text becomes selected and the hint changes.
- First run: shown while `first_run_seen` is false and it stays when providers answer; it
  states that Flashboard starts at login, with a working "Start at login" switch (calls
  `settings.update({autostart})`, rolls back and says why on an `io` error); Continue sets
  `first_run_seen`, moves focus to the overview heading, and the view never returns.
- `not_installed` copy names the PATH ("Flashboard can't find `codex` on its PATH").
- Reflow (D9.10): at 320 CSS px (a 320 px window at 100 %, and a 640 px window at 200 %)
  no clipped text and no horizontal scroll; the text-spacing run loses no content.
- **Focus kept**: with focus on a meter row of the second card, a snapshot that raises that
  card's severity reorders the cards and the same row still has focus; the same for a
  focused Copy button.
- `Esc` with a level-1 detail open closes it and stays on the overview; a second `Esc` does
  nothing there (the overview has no "back").
- Rapid snapshot updates (20 per second from the mock) cause no layout shift (CLS measured
  in Playwright = 0).

**Security rules.** D8.8 UI rules; only text from `copy.ts`, `format` and typed fields is
rendered; no dynamic HTML.

**Depends on.** T07, T14.

**Review checklist.**
- [ ] Matches the D9.3 wireframe and D9.5 state table; numbers right-aligned in tabular mono.
- [ ] Minimal at rest (level 0 only), detail one focus away, and the same detail in the
      accessible text; every item of D9.10 demonstrated.
- [ ] No invented numbers in any non-`known` state.
- [ ] Every sentence comes from `copy.ts`; tone per D9.9.
- [ ] Layout is fluid 320 → 1400 px; column rules as designed.
- [ ] No animation other than those in D9.7; reduced motion honoured.

---

## T10 — Provider detail with history chart, and Settings (wave 2, UI-design pass)

**Skills.** `impeccable`; `dataviz` before writing the chart; finish with `better-ui`.

**Goal.** D9.4 and D9.6.

**Owns.** `dashboard/src/screens/{ProviderDetail,Settings}.svelte`,
`dashboard/src/lib/components/detail/**` (`MeterHero`, `Chart`, `RangeSwitch`,
`MeterSelect`, `SourceNote`, `LiveCheckButton`), `dashboard/src/lib/chart/**` (pure
geometry), `dashboard/src/lib/components/settings/**`,
`dashboard/src/lib/detail/history.svelte.ts` (not under `lib/stores/`, which T07 owns),
their tests, `dashboard/e2e/screens/{detail,settings}.spec.ts` and baselines.

**Exposes.**
```ts
// lib/chart (pure, unit-tested)
export type Scale = { x(t: number): number; y(v: number): number };
export type Pad = { top: number; right: number; bottom: number; left: number };
export type Tick = { at: number; label: string };
export function makeScale(series: HistorySeries, width: number, height: number, pad: Pad): Scale;
export function stepPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // breaks at gaps; gapSeconds is always series.gap_s
export function bandPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // lo..hi
export function ticks(series: HistorySeries, range: HistoryRange, hour: HourCycle): { x: Tick[]; y: Tick[] };
export function summarize(series: HistorySeries, hour: HourCycle): string;                     // the role="img" label
export function nearest(points: HistoryPoint[], t: number): number;                            // index for the crosshair
export function pointText(p: HistoryPoint, unit: HistorySeries["unit"], hour: HourCycle): string;   // the crosshair slider's aria-valuetext
// lib/detail/history.svelte.ts
export const history: { load(q: HistoryQuery): Promise<void>; readonly state: "idle" | "loading" | "ready" | "empty" | "error"; readonly series: HistorySeries | null };
```

**Acceptance tests.**
- Chart geometry: step path for known points equals an expected path string; gaps split the
  path exactly where two points are further apart than `series.gap_s` (the UI has no gap
  constant of its own); band for rollups; empty series → empty state ("No history yet. It
  builds up as you use the provider."); single point renders a dot; values above 100 %
  extend the axis; reset hairlines from `series.resets`, and the summary omits the reset
  count when `resets` is empty (the quarter range; a 5-hour window over a month).
- Chart a11y: `role="img"` label from `summarize`; hidden data table has one row per point
  (capped at 600); the crosshair is a separate `role="slider"` control (D9.4): Left/Right,
  Home and End move it, `aria-valuenow` is the index and `aria-valuetext` is `pointText`;
  moving it writes **nothing** to the app's live region; pointer hover shows the same
  readout; axe clean.
- Range switch is a radio group with arrow keys; changing range/meter issues exactly one
  `getHistory` and cancels a stale response (out-of-order responses do not overwrite).
- Detail states: provider `unavailable`/`not_configured`/`unsupported` show the state block
  and no empty chart frame; `stale` shows the freshness note.
- Live check button exists only when `refresh.live_check` is true; it states the cost;
  `NotEnabled`/`Busy` handled; a non-null `refresh.live_check_error` is shown next to the
  button in `copy.ts` words and does not change the meters above.
- Detail view is level 2 of D9.1: every window in full (number, countdown and absolute time
  side by side, status), without needing hover.
- Settings covers every field of D7.4 `Settings` except `first_run_seen`: the notification
  master switch with the two level switches disabled (values kept) while it is off; "Start
  at login" **on by default**, rolling back with a reason on an `io` error; the single-key
  shortcuts switch, with the full key list in text under it (`R`, `,`, `1`–`6` with the
  provider each opens, `?`, `Esc`, Ctrl + plus / minus / zero; D9.10); the "Size" radio group (`zoom_pct`, five steps); "About and diagnostics" showing the name Flashboard and the version.
- Settings: every control reads and writes through `settings.update`; thresholds validate
  on blur (warn < critical, 50..100) with a specific message tied to the field by
  `aria-describedby` and `aria-invalid` (D9.10 Forms); every control has a visible label
  that is its accessible name; sliders and number fields stay
  in sync; bar tick marks elsewhere move live; "Saved" tick announced; a failed save rolls
  the control back and says so; enabling the live check opens the confirmation dialog
  (focus trapped, Esc cancels, focus returns); tray toggle disabled with a reason when
  `tray_available` is false; diagnostics list renders `SourceDiag`.
- Screenshots: detail (two meters, one meter, balance, each range, empty, error) and
  settings x themes x widths; reduced motion; forced colors.
- Zero axe violations on every state x theme of both screens; recorded focus sequence per
  screen; dialog focus trap and return; sliders operable by arrow keys and by their number
  field (no drag needed); targets ≥ 32 px.
- Keyboard walk-through of both screens recorded in the report.
- Type discipline (D10): zero findings in everything this task owns, tests and specs
  included; every component `lang="ts"` with typed props.

**Security rules.** D8.8 UI rules. Diagnostics render only the typed fields of
`Diagnostics` (no paths, no raw errors).

**Depends on.** T07, T14.

**Review checklist.**
- [ ] Step chart, no interpolation across gaps, thresholds labelled in text.
- [ ] No chart dependency added; SVG only; colours from tokens.
- [ ] Chart is usable by keyboard and described for screen readers.
- [ ] Settings apply immediately and can never store an invalid threshold pair.
- [ ] Live-check consent dialog wording states the cost plainly.

---

## T11 — Tauri shell wiring: commands, events, tray, window lifecycle (wave 3)

**Goal.** The thin shell of D5.2: wiring only, no business logic.

**Owns.** `dashboard/src-tauri/src/{app.rs,commands.rs,events.rs,tray.rs,window.rs}`,
`dashboard/src-tauri/tests/{commands_*,window_*,tray_*,manifest_*}.rs`. (`lib.rs` and
`main.rs` are T00's; the leaf modules are T15's.)

**Exposes / implements.** Everything that takes an app handle is generic over the runtime
(`R: tauri::Runtime`), because the tests use `tauri::test`'s mock runtime and a function
written for the default runtime does not accept it.
```rust
// commands.rs — exactly these six, names fixed by the capability file
// Every command returns the generated envelope `Outcome<T>` and always resolves (D7.3).
#[tauri::command] fn get_snapshot(svc: State<'_, ServiceHandle>) -> Outcome<Snapshot>;
#[tauri::command] async fn refresh(svc: State<'_, ServiceHandle>, target: RefreshTarget) -> Result<Outcome<()>, CommandError>;
#[tauri::command] async fn get_history(svc: State<'_, ServiceHandle>, query: HistoryQuery) -> Result<Outcome<HistorySeries>, CommandError>;
#[tauri::command] fn get_settings(svc: State<'_, ServiceHandle>) -> Outcome<Settings>;
#[tauri::command] async fn update_settings<R: Runtime>(app: AppHandle<R>, svc: State<'_, ServiceHandle>, patch: SettingsPatch) -> Result<Outcome<Settings>, CommandError>;
#[tauri::command] async fn get_diagnostics<R: Runtime>(app: AppHandle<R>, svc: State<'_, ServiceHandle>) -> Result<Outcome<Diagnostics>, CommandError>;   // ShellFacts: version + tray host asked now
// The outer `Result` exists only because Tauri demands one from an async command that borrows state; it is always `Ok`.
// events.rs
pub const EVENT_SNAPSHOT: &str = "snapshot"; pub const EVENT_NAVIGATE: &str = "navigate";
pub fn forward_snapshots<R: Runtime>(app: AppHandle<R>, rx: watch::Receiver<Snapshot>);   // emits only to the "main" window, only while it exists
// tray.rs
#[derive(Clone, PartialEq, Debug)]
pub struct TrayModel { pub rows: Vec<TrayRow>, pub icon: IconSpec, pub notifications: bool }
/// Pure: rows from core::text::tray_rows, icon from the headline.
pub fn model(snapshot: &Snapshot, settings: &Settings, now: UnixSeconds, local: &dyn LocalTime) -> TrayModel;
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), TrayError>;    // TrayError is a field-less enum; id and title "Flashboard"
/// Touches the menu and icon only when `next` differs from the model last applied (D7.7).
pub fn apply<R: Runtime>(app: &AppHandle<R>, next: TrayModel);
// window.rs
pub fn open<R: Runtime>(app: &AppHandle<R>, route: Route);                  // create (URL fragment: view, provider, theme; incognito; zoom; drag-and-drop handler off) or show + `navigate` + focus request
pub fn close_requested<R: Runtime>(app: &AppHandle<R>, host: &dyn TrayHost);   // tray host present now and close_to_tray: hide, destroy after 30 s; else quit
pub fn remember_size<R: Runtime>(app: &AppHandle<R>, w: u32, h: u32);       // debounced into the store's meta (WindowWidth/WindowHeight), through spawn_blocking
// app.rs
pub fn run();                                                               // the wiring below
```
`app::run` order: `hardening::apply()` (before any thread) → take the binary's path once
(`current_exe`, D9.13) → builder with the single-instance plugin registered first (a second
launch calls `window::open` unless its arguments contain `--hidden`) → in `setup` (which runs
after plugin initialisation, so a second process never gets here): build `ServiceDeps`
(engine_bridge paths, `SqliteStore`, `JsonSettingsStore`, `ProcRunner::system`,
`registry::build`, `registry::backfill_job`, `watch_file`, `CliProbe`, `SystemTimer`,
`SystemJitter`, `DesktopNotifier`) → `spawn(deps, rt)` and `watch_file(.., rt)` with Tauri's
runtime handle (`tauri::async_runtime::handle().inner()`, the tokio handle inside Tauri's
wrapper; `setup` is not inside a runtime) →
`autostart::reconcile` from the loaded settings (release builds only; on failure set
`autostart` off through the service and emit `DiagEvent::AutostartFailed`, never fatal) →
tray → a signal listener for `SIGTERM`, `SIGINT` and `SIGHUP` (shutdown, then exit, D8.8) →
window unless `--hidden` → run loop.
`ExitRequested` is prevented only when it carries no exit code; tray Quit awaits
`ServiceHandle::shutdown()` and then exits. Service visibility follows the window
(`set_visible`). Navigation locked to the app origin; new windows denied. The web view is
created in code on open (`incognito(true)`, `disable_drag_drop_handler()` so no dropped
file's path can reach the page (D8.8), size from the store's meta, zoom from
`settings.zoom_pct` and re-applied when it changes) and destroyed 30 s after close (D9.13).
`update_settings`: after the service accepts the patch, if `autostart` changed the shell
reconciles the entry; on failure it restores the previous value through the service and
returns the `io` error outcome. The tray's Notifications checkbox writes
`settings.notifications`; the shell re-applies the tray model when settings change
(`subscribe_settings`). `DiagEvent::WindowReady` is emitted when the first `get_snapshot`
of a newly created web view returns; the commands may take further `State<'_, …>`
parameters for shell state such as that window-creation instant (the six names and their
IPC arguments and results are what is fixed). `autostart::reconcile` returns T15's
field-less `AutostartError`, which the command maps to the `io` outcome.

**Acceptance tests.**
- `update_settings` with a failing reconcile returns `io` and the stored setting is the old
  value; a failing reconcile at start leaves `autostart` off.
- Tray (pure): `model` of two snapshots that differ only in `RefreshView` is equal; a
  changed row, level or `notifications` value makes it differ; a row's menu id maps back to
  its provider. That the menu is really rebuilt once is in the manual list.
- Second launch: without `--hidden` opens the window; with `--hidden` does nothing.
- Command layer with `tauri::test` mock runtime: each command resolves with an `ok` outcome
  carrying the handle's value; no command ever rejects for a service error (`Busy`,
  `NotEnabled`, a store failure and out-of-range thresholds are `error` outcomes); an unknown provider or a malformed
  meter id is rejected before the command body runs (D7.3; the client's mapping of that
  rejection is T07's test); payload JSON equals the bindings' shape.
- Snapshot tests: capability file, CSP, plugin list, absence of the `devtools` feature,
  `withGlobalTauri: false`, exactly six commands registered.
- Window lifecycle with the mock runtime and a fake `TrayHost` (the test sets Tauri's async
  runtime to a paused tokio runtime with `tauri::async_runtime::set`; that call panics the
  second time in one process, so each test binary that needs paused time has **one**
  `#[test]` that builds the runtime once and runs its cases in sequence): close → hidden →
  destroyed after 30 s (paused time); open recreates with the route and theme in the fragment; open
  on an existing window emits `navigate`; the zoom factor follows `zoom_pct`; `--hidden` creates no window; no tray host at
  close time → close quits; `--hidden` with no tray host stays resident; a host that appears
  after start is seen at the next close (nothing cached).
- Signals: the handler path (a function the test calls directly) shuts the service down and
  requests exit; the real-signal case is T12's smoke test.
- Check and report (D16): `WebviewUrl::App` keeps the fragment (else use the fallback of
  D7.3).
- Manual check list (recorded in the report), on this machine, in the real session with any
  installed Flashboard quit first (it holds the same single-instance name): tray icon,
  title (shown as a name, not as a label painted beside the icon, D9.11) and menu in the quickshell host, a menu row opens that provider, the menu is not
  rebuilt while only refresh bookkeeping changes; a warn and a critical notification
  appear once each with the right urgency; a second launch opens the window; the size is
  remembered; light and dark follow the OS in the real WebKitGTK view and the Theme setting
  overrides it, with no flash of the other theme when the window opens (D9.7, else the
  native background fallback); **log out and in: the app is running hidden, with Claude and Codex found**
  (autostart and its `PATH`); with `PR_SET_DUMPABLE=0` the window, clipboard copy and theme
  still work, and Orca (installed, D3.4) still reads the window (else fall
  back per D8.8 and report); after closing the window for 30 s no
  WebKit helper process remains and no directory was created under `~/.local/share` or
  `~/.cache` for the app; suspend past a window's reset and resume: the tray is correct
  within seconds; the app runs beside a live `jev-claude` session with no effect on it.

**Security rules.** D8.8 in full; D8.4 (no logging except `diag`; panic hook); D7.3 (no
extra command, no event from UI to Rust); notifications and tray text only from
`core::text`; the shell reads no file itself except through `fsutil`/stores.

**Depends on.** T08, T15 (and the event/command names used by T07's `tauri.ts`).

**Review checklist.**
- [ ] No logic in the shell that belongs in core/io (no formatting, no thresholds, no parsing).
- [ ] Exactly six commands; capability and CSP snapshots unchanged from T00.
- [ ] The window builder disables the drag-and-drop handler (a source test finds the call
      in `window.rs`; manual: dropping a file on the window does nothing).
- [ ] Web view destroyed when closed; service marked hidden; no timer in the shell except
      the 30 s destroy and the size debounce.
- [ ] Nothing spawns onto a runtime that is not there: the service and the watcher get
      Tauri's runtime handle; signals end in a clean shutdown.
- [ ] Only the single-instance plugin; `zbus` only in `bus.rs`, `notify_rust` only in
      `notifier.rs`; no `unsafe`.
- [ ] Every name the user sees says Flashboard; the identifier is `dev.jev.flashboard`.
- [ ] Tray works with menu-only interaction; no-tray fallback implemented.

---

## T12 — Security proofs, CI, end-to-end, budgets (wave 4)

**Skills.** Run `security-review` on the complete dashboard diff and fix or report every
finding.

**Goal.** Make every rule of D8 and D11 mechanically enforced, and finish `scripts/ci.sh`.

**Owns.** `scripts/ci.sh` (takes over from T00), `deny.toml` tuning (with lead approval for
any allow), `dashboard/crates/usage-io/tests/{architecture.rs,canary_e2e.rs,env_scrub.rs}`,
`dashboard/crates/usage-core/tests/ipc_audit.rs`, `dashboard/fuzz/**`,
`dashboard/e2e/smoke/**` (the driver-free real-binary test and its stub `codex`/`claude`
programs), `dashboard/e2e/webdriver/**`, `dashboard/e2e/screens/security.spec.ts`,
`dashboard/scripts/measure-idle.sh`, coverage configuration.

**Delivers.**
- Every proof in the table of D8.10 that is not already owned by T01–T11, in particular:
  the **architecture test** (allowed locations of `expose_secret` and `reqwest` (`http.rs`),
  process APIs (`proc.rs`), `zbus` (`bus.rs`), `notify_rust` (`notifier.rs`), the engine's
  crate identifiers (`engine_bridge.rs` and one manifest line, D5.1); forbidden file names,
  CLI home paths and `probe_all`/`path_from_env`; forbidden macros; the type-file rule; the
  three `[lints]` tables equal; the login commands in `copy.ts` equal
  `engine_bridge::login_command`),
  the **IPC audit** (key allow-list, forbidden key pattern, string-leaf vocabulary, over
  proptest-generated values of every IPC type and over the generated `.ts` files), the
  **end-to-end canary** (service + real adapters over stubs + real store in a temp state
  dir; search snapshots, all files' raw bytes, `diag` capture), the **no-network** Playwright
  test (any request other than app origin/IPC fails; CSP violation events fail).
- Fuzz targets for the seven entry points of D11.2 with seed corpora from the fixtures, and
  the stable byte-level proptests that mirror them.
- **Real-binary smoke test** (stage 10 of D11.4, no driver): start the `dist` binary on the
  current Wayland display under `dbus-run-session --config-file` with the activation-free
  `e2e/smoke/session-bus.conf` (D8.8: a private bus, so a resident Flashboard cannot
  swallow the launch, and no notification daemon or portal can be started by it), with `HOME`, every `XDG_*_HOME` and
  `XDG_RUNTIME_DIR` inside a temp tree (0700) containing a synthetic, canary-laced
  `usage.jsonl`, `WAYLAND_DISPLAY` set to the absolute path of the real socket, and a `PATH`
  that contains only stub `codex` and `claude` programs (no real CLI is ever run; the
  autostart entry lands in the temp config home). Assert from stderr `diag` lines and the
  temp tree: `Started`; a successful Claude-log and Codex fetch; `WindowReady` within the
  D10 budget; a new sample within 3 s of an appended row; rows in `history.db`; file modes;
  no canary anywhere in the tree or on stderr; no WebKit data directory; nothing new outside
  the tree (the **names** under the real `~/.config/autostart`, `<state_dir>/dashboard*`,
  `~/.local/share/dev.jev.flashboard*` and `~/.cache/dev.jev.flashboard*` are the same before
  and after; sizes and times are not compared, a live `jev-claude` changes those); clean
  exit on SIGTERM with no child left (the stub `codex` is made to hang for this case).
- WebDriver e2e (stage 13, **required**, D1a; `tauri-driver` + `WebKitWebDriver` from
  `webkitgtk-6.0`): the same isolated launch; assert the overview shows the fixture numbers
  in the real WebKitGTK view; open detail and settings by keyboard; change a threshold;
  verify `settings.json`; run axe inside the real view on each screen. Without the driver
  the stage prints NOT RUN with the install command; NOT RUN fails `--release` and is never
  reported as a pass (D11.4). Check and report the two D16 assumptions the first time it
  runs (the 6.0 driver drives the 4.1 app; automation with the incognito view).
- `measure-idle.sh`: launches the binary hidden under `dbus-run-session` against the temp
  tree, measures the fetch-free window from 60 s to 660 s after launch (D11.3) and
  reports CPU time, wakeups (voluntary context switches) and RSS, each **summed over the
  process tree and all threads** (`/proc/<pid>/task/*/status`), and the binary size, against
  the D11.3 budgets; the same run on T00's empty scaffold gives the floor, printed beside
  the app's figures. `WindowReady` in the smoke test is taken from the second launch in the
  same temp tree (D11.4).
- `ci.sh`: all thirteen stages of D11.4, `--bootstrap` (no root: cargo tools, pinned
  nightly, `tauri-driver`, Playwright Chromium), `--release`, a final table with
  PASS/FAIL/SKIPPED/NOT RUN per stage and total time; non-zero exit on any FAIL (and on
  SKIPPED or NOT RUN with `--release`, except the advisory fuzz stage 12, D11.4). Stage 2
  builds the UI before any cargo command compiles the shell, and fmt and clippy are scoped
  to the three app crates (D11.1, D11.4).
- Accessibility gate: stage 6 and stage 8 fail on any axe violation, on the contrast test,
  and on the keyboard-path and live-region tests of D9.10; a table in the report maps every
  bullet of D9.10 to the test (or manual step) that checks it.

**Acceptance.** `scripts/ci.sh` green on this machine; a deliberate violation of each rule
(a `println!`, a second `expose_secret`, an extra IPC field named `email`, a string leaf, an
extra capability permission, a remote `<img>`, a `style=""` attribute, a `localStorage`
call, a low-contrast token, an unnamed icon button, and for the type discipline of D1a: an `any`,
an `unknown`, a `never`, an `as` assertion, a non-null `!`, a `@ts-expect-error`, a
`catch (e)`, a type predicate, a `JSON.parse`, an inferred `any`, an `as` in Svelte markup,
an `eslint-disable` comment, a `serde_json::Value`, an `unwrap()`, a second float cast, an
`allow` outside the two of D11.1, a `std::io::Result` in a `pub` signature and an
`unsafe` block) makes the matching stage fail
(demonstrated in the report, not committed); coverage thresholds met or the gap reported per
file.

**Security rules.** All of D8. This task may not weaken a rule to make a test pass; a rule
that cannot be met is escalated to the lead.

**Depends on.** T01–T11, T14, T15; final run after T13 is merged (section 1).

**Review checklist.**
- [ ] Each row of D8.10 and each bullet of D9.10 maps to a named, passing test or a named
      manual step (tables in the report).
- [ ] Mutation check done: each proof fails when its rule is broken.
- [ ] `ci.sh` has no silent skip; engine stage first and unchanged; stage 6 runs
      `check-types.ts` and ESLint with `--no-inline-config --max-warnings 0`.
- [ ] `cargo deny`/`audit`/`pnpm audit` clean, or each exception justified and time-boxed.
- [ ] Idle budgets measured and reported with numbers.

---

## T13 — Packaging and final UI polish audit (wave 4, UI-design pass)

**Skills.** `impeccable` (audit, then polish), `better-ui`.

**Goal.** Installable on Arch, and a last expert pass over the whole UI against D9.

**Owns.** `dashboard/scripts/install.sh`, `dashboard/packaging/dev.jev.flashboard.desktop`,
`dashboard/src-tauri/icons/**` (app icon set, designed here), `dashboard/README.md`
(install, autostart, optional Hyprland window rule, privacy summary, troubleshooting),
`dashboard/CHANGELOG.md`, `dashboard/docs/screen-reader-script.md`; and, for the polish pass only, edits to UI files under
`dashboard/src/**` after T09/T10 are merged (list every touched file in the report; no
behaviour or contract changes; baselines regenerated and reviewed).

**Delivers.**
- D12 build and install flow; `install.sh [dest]` and `--uninstall`; the binary is replaced
  by rename (a running instance is never half-replaced); desktop entry
  validated with `desktop-file-validate` (`Name=Flashboard`, icon `dev.jev.flashboard`,
  absolute `Exec`); icons in hicolor sizes; `--uninstall` also removes the autostart entry
  the app wrote; the script touches nothing else and the root `scripts/install.sh` is
  unchanged.
- An `impeccable` audit report against D9.1 and D1a, word by word: **minimal** (chrome,
  palette count, whitespace, one hierarchy per card, nothing decorative), **detailed** (the
  three levels present; level 1 identical on hover and focus; nothing reachable only by
  pointer or only by sight), **beautiful** (type scale, spacing rhythm, alignment of
  numerals, radii, both themes as equals, motion), **accessible** (every bullet of D9.10),
  plus state coverage, copy tone, 320 px and 200 % zoom; each finding fixed or listed for
  the lead; a `better-ui` pass (concentric radii, optical alignment, hit areas, surface
  depth).
- The **screen-reader script** (`dashboard/docs/screen-reader-script.md`, owned here): for
  every screen and state, the keys to press and what Orca must say, in order, each line with
  a box to tick and room for "said instead". The worker delivers the script and a
  keyboard-only walk-through; the **pass itself is run by the user with the lead** (D1a,
  D11.5): the user (Orca is installed, D3.4) follows the script on the installed app in the real
  WebKitGTK view and confirms or corrects each line; every difference is a finding. The
  first lines check the D16 assumption about `role="meter"`. v1 is not done until this pass
  has no open finding.
- README: install, autostart (on by default, how to turn it off, what file it writes), the
  `PATH` note, optional Hyprland window rule, privacy summary with the honest limits of D8
  (including what the login probe runs and reads, and the mise wrapper note), troubleshooting.

**Acceptance.** Fresh install into a temp prefix runs; uninstall leaves no file (the
autostart entry included); the app launches from the desktop entry and with `--hidden`;
`scripts/ci.sh` still green with regenerated baselines (T12 makes the `--release` run on
top); bundle and idle budgets still met; zero axe violations after the polish; the audit
report shows each of the four words of D1a met.

**Security rules.** The install script uses no `sudo`, no network, no `curl | sh`, writes
only under the given prefix and `~/.local/share`; the README's privacy section states
exactly what D8 guarantees and its honest limits.

**Depends on.** T09, T10, T11 (runs alongside T12; T12 owns `ci.sh`, T13 does not edit it).

**Review checklist.**
- [ ] `scripts/install.sh` byte-identical to before the project.
- [ ] Desktop entry valid; icon legible at 16–128 px in light and dark.
- [ ] Polish diff changes presentation only; no new dependency; screenshots reviewed.
- [ ] README privacy claims match D8 exactly (no overclaiming).
- [ ] The screen-reader script exists and is specific enough for the user to follow alone;
      the pass with the user is scheduled by the lead and its findings are closed (D11.5).
- [ ] Nothing says "Jev Dashboard" or `dev.jev.dashboard`.

---

## T14 — UI primitives (wave 1, UI-design pass; split from T07)

**Skills.** `impeccable` while building; finish with a `better-ui` pass.

**Goal.** Every reusable component of D9, complete in states, keyboard and ARIA, so the
screens only compose.

**Owns.** `dashboard/src/lib/components/primitives/**`, their component tests,
`dashboard/e2e/screens/primitives.spec.ts` with its baselines.

**Consumes.** T07's tokens, `format`, `clock` and the Playwright harness (T00's stubs,
the harness stub included, until T07 is merged; final test run after it, section 1). No IPC, no stores other than `clock`.

**Exposes.** Each with documented props: `Meter` (bar + thresholds + hatch for critical +
`role="meter"`, a required accessible name, `tabindex="0"` with the approved `svelte-ignore`
of D9.3, `aria-valuenow` clamped to 0..100, the full level-1 text as `aria-valuetext`; a
`MeterText` variant for a row without a number), `Detail` (the level-1 disclosure of D9.1:
opens on hover **and** focus, hoverable, `Esc` closes without moving focus, never covers its
trigger), `StatusGlyph`, `Card`, `Button`, `IconButton`, `CopyCommand`, `Countdown`
(re-renders from `clock`), `Freshness`, `Skeleton`, `RadioGroup`, `Switch`, `Slider`,
`NumberField` (with an error slot wired to `aria-describedby`/`aria-invalid`), `Dialog`,
`Icon` (inline SVG). The header and the live region are T07's (`components/shell/`), not
primitives. No primitive nests one interactive element in another.

**Acceptance tests.**
- Every primitive: Testing Library tests for each state and keyboard behaviour; **zero axe
  violations**; screenshot per state x theme (`primitives.spec.ts`), plus reduced-motion and
  forced-colors runs.
- `Detail`: opens on focus and on hover with the same content; stays while hovered; `Esc`
  closes and focus stays on the trigger; its text equals the trigger's `aria-valuetext`
  detail; it does not overlap the trigger at 320 CSS px (both cases of D9.10); `Esc` on an
  open `Detail` is consumed (the view does not go back, D9.1).
- `Meter`: value 104 % gives `aria-valuenow="100"` and the real figure in the text; no
  number → no `role="meter"`.
- Target size: every interactive primitive measures ≥ 32 x 32 px hit area (Playwright
  bounding boxes).
- `CopyCommand`: copies the given constant, shows and announces "Copied"; when the clipboard
  API rejects, the text becomes selected.
- `Dialog`: focus trap, `Esc`, focus return.
- Type discipline (D10): zero findings; every primitive's props are a named, exported type.

**Security rules.** D8.8 UI rules (no `style=` strings, no `{@html}`); text only from props.

**Depends on.** T00 (uses T07's tokens, `clock` and harness).

**Review checklist.**
- [ ] Status is never colour-only (glyph + word + pattern present in `Meter`).
- [ ] `Meter` and `Countdown` have correct accessible text; exactly one `svelte-ignore`, on
      the meter row, with its reason.
- [ ] Minimal (D9.1): no shadows on cards, no decorative icons, one accent.
- [ ] Level 1 is reachable and dismissible by keyboard and identical for hover and focus.
- [ ] Tokens only; no magic numbers; nothing sets a `style` attribute.

---

## T15 — Shell leaf modules: tray icon, autostart, hardening, notifier, bus, local time (wave 1; split from T11)

**Goal.** The shell's self-contained modules, none of which needs the service, so T11 is
wiring only.

**Owns.** `dashboard/src-tauri/src/{tray_icon.rs,autostart.rs,hardening.rs,localtime.rs,
bus.rs,notifier.rs,cli.rs}`, `dashboard/src-tauri/tests/{tray_icon_*,autostart_*,
hardening_*,notifier_*,localtime_*,cli_*}.rs`.

**Exposes.**
```rust
// tray_icon.rs (pure)
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct IconSpec { pub used_step: u8 /* 0..=20, 5 % steps */, pub level: Severity }
pub fn render(spec: IconSpec, size: u32) -> Vec<u8>;                        // RGBA, D9.11 shapes; integer arithmetic only (no float cast, D11.1)
// bus.rs — the only module that uses zbus
pub trait TrayHost: Send + Sync { fn present(&self) -> bool; }              // injected, so window tests need no bus
pub struct SessionBusTrayHost; impl TrayHost for SessionBusTrayHost        // org.kde.StatusNotifierWatcher owned and IsStatusNotifierHostRegistered; asked each time, never cached
// notifier.rs — the only module that uses notify_rust
pub trait NotificationSink: Send + Sync { fn deliver(&self, title: &str, body: &str, critical: bool) -> bool; }   // the seam to notify_rust
pub struct NotifyRustSink; impl NotificationSink for NotifyRustSink         // production sink: app name Flashboard, desktop-entry hint, urgency (D9.12)
pub struct DesktopNotifier; impl DesktopNotifier { pub fn new(local: Arc<dyn LocalTime>, clock: Arc<dyn Clock>, sink: Arc<dyn NotificationSink>) -> Self; }
impl Notifier for DesktopNotifier   // title/body from core::text::notification(alert, now, local, hour); urgency by level; hands the alert to its own worker thread and returns
// autostart.rs
pub fn desktop_entry(exec: &str) -> String;                                 // pure; Name=Flashboard, Exec=<quoted exec> --hidden (D9.13 quoting)
/// Makes <config_home>/autostart/dev.jev.flashboard.desktop match `enabled` (atomic, 0600; rewrites when Exec differs).
/// `exec` must be valid UTF-8 and an existing file, else `InvalidExec` and nothing is written.
/// File work goes through `io::fsutil` (D8.4: no `std::io::Error` in a signature).
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum AutostartError { InvalidExec, Write }
pub fn reconcile(enabled: bool, exec: &Path, config_home: &Path) -> Result<(), AutostartError>;
// hardening.rs
pub fn apply();                                                             // umask 077, RLIMIT_CORE 0, PR_SET_DUMPABLE 0, panic hook (base name and line only)
// localtime.rs
pub struct SystemLocalTime; impl LocalTime for SystemLocalTime             // chrono Local
// cli.rs
pub struct Args { pub hidden: bool }  pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Args;   // unknown arguments are ignored
```

**Acceptance tests.**
- `tray_icon::render`: golden-image tests for representative specs (0, 50, 80, 95, 100 % x
  levels); property: every pixel buffer has length `size*size*4`, alpha-premultiplied
  correctly, deterministic.
- `autostart::desktop_entry` golden text (validated with `desktop-file-validate`); a path
  with a space, a `"`, a `$` and a `%` round-trips through the Desktop Entry quoting rules;
  `reconcile` in a temp config home: on creates the file 0600, off removes it, a changed
  `exec` rewrites it, repeated calls change nothing, an unwritable directory returns
  `Write` and leaves no partial file, a path ending in " (deleted)" or missing writes nothing.
- `hardening::apply`: after the call, a created file is 0600, the core limit is 0 and the
  process is not dumpable, read back with `rustix::process::dumpable_behavior()`
  (`/proc/self/status` has no such field) (test in a child process); panic hook output contains a base
  name and a line, no `/`, and not the payload (child process with a canary payload).
- `DesktopNotifier` with an injected sender that blocks: `notify` returns immediately; the
  sender receives the title and body of `core::text::notification` for the given hour
  cycle, and `critical` only for the critical level; a failing sender emits
  `DiagEvent::NotifyFailed` and nothing else.
- `SystemLocalTime` agrees with a fixed-zone calculation when `TZ` is set in a child process.
- `cli::parse`: `--hidden` anywhere; nothing else changes behaviour.

**Security rules.** D8.8 (`hardening`), D8.4 (panic hook; notification text only from
`core::text`), D9.13 (the autostart entry is the one file written outside the state
directory). `zbus` only in `bus.rs`, `notify_rust` only in `notifier.rs`; no `unsafe`.

**Depends on.** T00 (uses T02's `text::notification`).

**Review checklist.**
- [ ] `render` is pure and matches D9.11 (level is a shape, not only a colour).
- [ ] `Exec=` is quoted per the specification; no path is written that does not exist.
- [ ] The notifier can never block its caller; no notification text is built here.
- [ ] `hardening::apply` uses `rustix` only.

---

## 2. Lead review protocol (for every task)

1. Run `scripts/ci.sh`. Read the diff in full; check file ownership against this document.
2. Walk the task's checklist; for security rules, open the named test and confirm it would
   fail if the rule were broken.
3. Search the diff for: new dependencies, `allow(` attributes, `String` in error or IPC
   types, `unwrap`/`expect`, `Value`, `unsafe`, and in TypeScript `any`, `unknown`, `never`,
   ` as `, `!.`, `@ts-`, `catch (`, ` is ` in a return type, `declare`, `JSON.parse`, `eslint-disable`, `svelte-ignore` (one approved, D9.3), `io::Result` in a `pub` signature, new file names read from disk, new network hosts, new commands
   or permissions, timers and loops.
4. For UI tasks: review every baseline screenshot in both themes at 360 and 1200 px, tab
   through the screen, read the copy aloud; check the four words of D1a (minimal, detailed,
   beautiful, accessible) against D9.1 and every bullet of D9.10.
5. Reject with specific, reproducible findings; accept only when D11.5 is fully met.
6. Repeat the review round until a full round finds nothing (D1a). Only then is the task done.
