# Flashboard (the Jev usage dashboard): work breakdown

Read `DESIGN.md` first. It is normative; this file says who builds which part of it.
Section numbers below ("D4.2") refer to `DESIGN.md`. D1a (the user's decisions) overrides
everything, including this file.

## 0. Rules for every worker

1. **Ownership is exclusive.** Edit only the files your task owns. Every file in the tree is
   created by T00 (as a compiling stub with its final public signatures) and then owned by
   exactly one later task. If you need a change in a file you do not own, or in a **frozen**
   file (list below), stop and send the lead a contract change request; do not edit it.
2. **Frozen after T00** (lead approval to change): every `Cargo.toml`, `Cargo.lock`,
   `deny.toml`, every `lib.rs`/`mod.rs`, `src/embed.rs`, `dashboard/package.json`,
   `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`, `vitest.config.ts`,
   `playwright.config.ts`, lint configs,
   `dashboard/src-tauri/{build.rs,tauri.conf.json,capabilities/}`, the core type files
   `time.rs quantity.rs label.rs provider.rs meter.rs usage.rs port.rs secret.rs ipc.rs`,
   `dashboard/src/lib/ipc/bindings/`, `dashboard/fixtures/ipc-allowlist.txt`. Type
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
6. Lints are `-D warnings`. No `unwrap`/`expect`/`panic!`/indexing/`as` outside tests, no
   `println!`/`eprintln!`/`dbg!`, no `unsafe`, no error type that carries a string, no
   `TODO`. A lint `allow` needs a one-line reason and lead approval.
7. Done means D11.5, including its rule that review rounds repeat until one finds nothing.
   Report with: files changed, tests added (names), how each security rule of the task is
   proven, anything you could not verify, and every assumption of D16 you confirmed or
   found false.
8. The engine (`src/`, root tests, `scripts/install.sh`) must stay green and untouched
   except `src/embed.rs` and one `pub mod embed;` line (T00).
9. Names (D1a): the product is **Flashboard**, the identifier `dev.jev.flashboard`, the
   binary and crates `jev-dashboard`/`jev-usage-*`. No file, string or test says
   "Jev Dashboard" or `dev.jev.dashboard`.
10. UI tasks (T07, T09, T10, T13): D9.1 and D9.10 are acceptance criteria, not guidance.
    Minimal chrome and palette, the three disclosure levels, and every accessibility item
    are checked by the lead; a UI task with an open accessibility finding is not done.

## 1. DAG and waves

```
W0   T00 scaffold + contract
      │
W1   T01 parsers   T02 policy   T03 claude log   T04 subprocess   T05 deepseek   T06 persistence   T07 UI foundation
      └──────┬───────┴──────┬──────┴───────┬───────┴───────┬───────┴──────┘            │
W2          T08 usage service (needs T01–T06)                          T09 overview   T10 detail + settings   (need T07)
             │                                                            └──────┬───────┘
W3          T11 Tauri shell (needs T08; UI contract from T07)                    │
             └───────────────────────────────┬───────────────────────────────────┘
W4          T12 security proofs + CI + e2e + budgets        T13 packaging + UI polish audit
```
All tasks in a wave run in parallel, each in its own worktree. A wave starts when the
previous wave is merged and `scripts/ci.sh` is green. Inside wave 1 there is one soft order:
T03, T04, T05 call T01's parsers and T06 calls T02's history rules. They compile against
T00's stubs from the start, so nobody waits; the lead merges T01 and T02 first, and T03–T06
rebase on them before their final test run (their fixture-driven tests cannot pass against
the neutral stubs). Inside wave 4 there is the same kind of soft order: T13's polish pass
regenerates screenshot baselines, so the lead merges T13 first and T12 makes its final
`scripts/ci.sh` run on top of it.

| task | wave | UI-design pass | skills to load |
|---|---|---|---|
| T00 | 0 | no (creates token file skeleton only) | none |
| T01–T06, T08 | 1–2 | no | none |
| T07 | 1 | **yes, primary** | `frontend-design` then `impeccable` |
| T09 | 2 | **yes** | `impeccable`, finish with `better-ui` |
| T10 | 2 | **yes** | `impeccable`, `dataviz` for the chart, finish with `better-ui` |
| T11 | 3 | tray icon only | none (follow D9.11 exactly) |
| T12 | 4 | no | `security-review` on the final diff |
| T13 | 4 | **yes, audit** | `impeccable` (audit + polish), `better-ui` |

`artifact-design` is not used anywhere in this project.

---

## T00 — Scaffold and shared contract (wave 0, one worker, strongest model)

**Goal.** Create the workspace, every crate, every file, the complete domain and IPC types,
the engine facade, the generated TypeScript bindings, the UI skeleton, the fixtures and a
first `scripts/ci.sh`, so that everything compiles and every later task only fills bodies.

**Owns (creates).**
- Root: the `[workspace]` (with `exclude = [".claude", "dashboard/fuzz"]`, D5),
  `[workspace.lints]`, `[profile.dist]` tables in `Cargo.toml` (nothing else in that file
  changes), `Cargo.lock`, `deny.toml`, `src/embed.rs`, one line `pub mod embed;` in
  `src/lib.rs`, `scripts/ci.sh` (stages 1–7 and 9 of D11.4 working; the rest present and
  reporting SKIPPED with a reason), `.gitignore` additions.
- `dashboard/crates/usage-core/**`: manifests, `lib.rs`, the frozen type files **complete
  with tests** (`time quantity label provider meter usage port secret ipc`), and stubs for
  `parse/{claude_log,claude_event,codex,deepseek}.rs`, `assess.rs alerts.rs summary.rs
  history.rs text.rs settings.rs view.rs` (types complete, functions returning a neutral
  value: `None`, `Err(Unavailable::Internal)`, an empty vec; no `todo!`, no lint allows).
- `dashboard/crates/usage-io/**`: manifests, `lib.rs`, **complete** `engine_bridge.rs`,
  `clock.rs`, `diag.rs`; stubs for `fsutil tail watch proc http secrets store settings_file
  timer registry service` and `sources/{claude_log,claude_live,codex,deepseek,placeholder}.rs`
  (`placeholder.rs` complete).
- `dashboard/src-tauri/**`: `Cargo.toml`, `build.rs` (AppManifest commands, D8.8),
  `tauri.conf.json` (CSP, security flags, identifier `dev.jev.flashboard`, product name
  `Flashboard`, `app.windows: []`, no bundle targets), `capabilities/main.json`, a `main.rs`
  that creates the `main` window in code and shows it empty, stub modules `commands events
  tray tray_icon notifier window autostart hardening localtime bus cli`.
- `dashboard/` UI: `package.json` (exact versions, scripts `dev build check lint test
  test:screens tauri`), `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`
  (`build.assetsInlineLimit: 0`), `vitest.config.ts`, `playwright.config.ts` (Chromium
  project; the static server sends the CSP of `tauri.conf.json` as a header),
  ESLint/stylelint/Prettier configs with the rules of D8.10 (raw-HTML sinks, web storage,
  `style=` attributes) and D9.7, `index.html`, `src/main.ts`, stub `src/App.svelte`, stub
  screens
  `src/screens/{Overview,ProviderDetail,Settings}.svelte`, empty
  `src/lib/design/tokens.css`, `src/lib/ipc/bindings/*` (generated).
- `dashboard/fixtures/`: keep `codex-app-server/` (already recorded), add
  `claude-gateway/{sample.jsonl,canary.jsonl,rotation/…}` (synthetic rows shaped like D3.2,
  canary per D8.10), `deepseek/{balance_ok.json,balance_two_currencies.json,
  unauthorized.json}`, `format-vectors.json` (≥ 60 cases for D9.9), `ipc-allowlist.txt`.

**Interfaces to expose (exactly as written in the design).**
- D4.1–D4.4 types and ports, D7.4 IPC types (`core::ipc`), D8.5 `Secret<T>`, D7.8
  `LocalStamp`/`LocalTime` (in `core::text`), D5.1 `jev_router::embed`.
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
  // io::engine_bridge
  pub enum Readiness { Ready, NotInstalled, NotLoggedIn, NotConfigured, Unknown }
  pub struct EngineFacts { /* private */ }
  impl EngineFacts {
      /// Pure, tested. Keeps the `State` and the plan label per provider and drops every
      /// other field of `Availability` (`detail`, `models`). `NoKey` maps to `NotConfigured`.
      pub fn from_availability(a: &[embed::Availability]) -> Self;
      pub fn unknown() -> Self;                                         // before the first probe: every provider `Unknown`
      pub fn readiness(&self, p: ProviderKey) -> Readiness;
      /// The engine's plan, ASCII-lower-cased with `-` and space turned into `_`, through `Label::parse`; else None.
      pub fn plan(&self, p: ProviderKey) -> Option<Label>;
  }
  pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = EngineFacts> + Send + 'a>>;
  /// The one seam to the engine's probe (which runs real CLIs), so the service is testable.
  pub trait EngineProbe: Send + Sync { fn probe(&self) -> ProbeFuture<'_>; }
  pub struct SystemProbe;  impl EngineProbe for SystemProbe             // embed::probe_all(), mapped
  pub struct FixedProbe(pub EngineFacts);  impl EngineProbe for FixedProbe   // feature "test-support"
  pub struct FactsCell(/* RwLock<EngineFacts> */);                      // shared read handle
  impl FactsCell { pub fn new(f: EngineFacts) -> Arc<Self>; pub fn get(&self) -> EngineFacts; pub fn set(&self, f: EngineFacts); }
  pub fn usage_log_path() -> Option<PathBuf>;                           // embed::path_from_env("JEV_USAGE_LOG","usage.jsonl") (a String) as a PathBuf
  pub fn dashboard_dir() -> Option<PathBuf>;                            // <state_dir>/dashboard
  pub fn env_file() -> Option<PathBuf>;                                 // embed::env_file_path(...)
  // io::clock
  pub struct SystemClock;  impl Clock for SystemClock
  pub struct FakeClock;    impl FakeClock { pub fn at(ms: u64) -> Arc<Self>; pub fn advance(&self, d: Duration); pub fn set(&self, ms: u64); }   // feature "test-support"
  // io::diag
  pub enum DiagEvent { SourceOk { source: SourceId, took_ms: u32 }, SourceFailed { source: SourceId, reason: Unavailable },
                       StoreFailed { error: StoreError }, LogRotated, WatchLost, WatchArmed, ClockChanged,
                       ProbeDone { took_ms: u32 }, BackfillDone { samples: u32 }, WindowReady { took_ms: u32 },
                       NotifyFailed, AutostartFailed, Started, Stopping }
  // io::timer (D7.1)
  pub enum Wake { Due, ClockChanged }
  pub trait WallTimer: Send + Sync { fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_>; }
  pub struct SystemTimer;  pub struct FakeTimer;                         // FakeTimer: feature "test-support"
  // io::service (types only; T08 writes the bodies)
  pub trait Jitter: Send + Sync { fn next(&self) -> f64; }
  pub struct Backfill { pub provider: ProviderKey, pub run: Box<dyn FnOnce() -> Result<u64, StoreError> + Send> }
  pub struct ShellFacts { pub app_version: &'static str, pub tray_available: bool }
  // io::store (types only; T06 writes the bodies)
  pub enum MetaKey { BackfillDone, LastPruneAt, WindowWidth, WindowHeight }
  pub fn emit(e: DiagEvent);                                            // stderr, one line, numbers and enum names only
  pub fn capture() -> CaptureGuard;                                     // feature "test-support": collect instead of print
  ```
- `usage-io` has a Cargo feature `test-support` (FakeClock, FakeTimer, FixedProbe, diag
  capture, proc/http test constructors) enabled only by dev-dependencies and tests.
- IPC names: `ProviderKey` serialises exactly as `as_str()` (`deepseek`, not `deep_seek`);
  `Setup` has two variants (`use_gateway`, `add_deepseek_key`), D4.3.
- Bindings: `cargo test -p jev-usage-core --features bindings export_bindings` writes
  `dashboard/src/lib/ipc/bindings/*.ts`; `ci.sh` fails on drift.

**Acceptance.**
- `cargo build --all-targets --locked` and `cargo test --locked` at the root: unchanged
  behaviour, all existing engine tests pass; `scripts/install.sh` untouched and working.
- `cargo test -p jev-usage-core -p jev-usage-io --locked` passes; `cargo build -p
  jev-dashboard --locked` passes; `pnpm --dir dashboard install --frozen-lockfile && pnpm
  --dir dashboard build && pnpm --dir dashboard check && pnpm --dir dashboard lint` pass;
  `pnpm --dir dashboard tauri build --no-bundle -- --profile dist` produces a binary that
  opens an empty window.
- Unit and property tests for the frozen types: `Percent` (saturation, NaN/negative
  rejected, ordering), `Money::parse_decimal` (exact, rejects exponent/garbage/overflow,
  round-trip), `Label::parse` (charset, length 1..=40), `Meters::new` (rejects empty, > 8,
  duplicate ids), `MeterId::key`/`parse_key` round-trip for arbitrary values,
  `WindowKind::from_minutes`.
- `static_assertions`: `Secret<String>` implements none of `Debug Display Clone Serialize
  PartialEq`. A source-scan test (Rust has no compile-time "contains no String" check):
  the frozen type files contain no `String`, `&str`, `Box<str>` or `Cow` outside `label.rs`.
- `EngineFacts::from_availability` table test over every engine `State`; a test that an
  `Availability` whose `detail`, `models` and `plan` hold the canary yields facts whose
  Debug output has no canary; plan normalisation (`max_5x`, `Max`, `pay-per-token` → `None`
  only when it still fails `Label::parse`).
- `SystemTimer`: a deadline 50 ms ahead fires `Wake::Due`; a past deadline fires at once.
- `scripts/ci.sh` runs and prints the stage table.
- Workspace: `cargo metadata` succeeds from inside a pre-existing agent worktree under
  `.claude/worktrees/` (the `exclude` works), and `cargo build` at the root builds only the
  engine.
- Confirm and report (D16): the pinned versions of Tauri 2, Svelte 5, Vite; that `tauri
  build -- --profile dist` lands in `target/dist/`; whether `tauri-plugin-notification` has an
  urgency setting (if it does, tell the lead before using `notify-rust`).

**Security rules.** D8.4 (no string in errors, `diag` typed), D8.5, D8.8 (capabilities, CSP,
plugin list exactly as designed), D8.9 (exact pins, lockfiles, `deny.toml`, npm script
blocking and release-age rule), fixtures contain no real data.

**Depends on.** nothing.

**Review checklist (lead).**
- [ ] Root `Cargo.toml` diff is only the three tables; engine build and tests identical.
- [ ] `default-members = ["."]`; `exclude` has `.claude` and `dashboard/fuzz`; `resolver = "2"`;
      `profile.dist` separate from `release`.
- [ ] `src/embed.rs` is re-exports only.
- [ ] Every IPC type matches D7.4 field for field; bindings generated, not hand-edited.
- [ ] No type in core can hold a free string except `Label`.
- [ ] Direct dependency lists match D8.9 exactly; versions exact; no git sources.
- [ ] `tauri.conf.json`, capability file and `build.rs` match D8.8 character for character.
- [ ] Stubs compile without `todo!`/`unimplemented!`; signatures match this file.

---

## T01 — Parsers (wave 1)

**Goal.** The four allow-list parsers of `jev-usage-core::parse`, proven unable to surface
anything but numbers, closed enums and `Label`s.

**Owns.** `dashboard/crates/usage-core/src/parse/{claude_log,claude_event,codex,deepseek}.rs`,
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
`might_have_sample` tests for the bytes `"ratelimit":{"`.

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
- **Canary**: `claude-gateway/canary.jsonl` and `account_read.response.json` fed to every
  parser: `format!("{:?}")` of every result contains neither `JEVCANARY` nor
  `canary-email`. Property test: arbitrary JSON with the canary string inserted at random
  keys/values/depths, same assertion, for all four parsers.
- Property: no panic and bounded output for arbitrary bytes (also invalid UTF-8, deep
  nesting 10 000 levels, 1 MiB strings, huge numbers, NaN-like tokens, duplicate keys).
- Property: a generated valid sample serialised to a log row with random extra keys parses
  back to the same sample.
- `might_have_sample` has no false negatives (property: `parse_line(l).is_some()` implies
  `might_have_sample(l)`).

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
pub fn severity(used: Percent, t: &Thresholds) -> Severity;
// alerts (D7.6)
pub fn step(memory: &AlertMemory, provider: ProviderKey, reading: &Reading, now: UnixMillis,
            settings: &Settings) -> (AlertMemory, Vec<Alert>);
// summary
pub struct Headline { pub provider: ProviderKey, pub meter: MeterId, pub used: Percent,
                      pub severity: Severity, pub resets_at: Option<UnixSeconds> }
pub fn headline(providers: &[(ProviderKey, Usage)], now: UnixMillis, settings: &Settings) -> Option<Headline>;
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
pub struct TrayRow { pub provider: ProviderKey, pub text: String }                   // a row opens its provider (D9.11)
pub fn tray_reset(at: UnixSeconds, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String;   // "14:00" | "Thu 02:00" | "16 Dec"
pub fn tray_rows(snapshot: &Snapshot, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> Vec<TrayRow>;
pub fn notification(alert: &Alert, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> (String, String); // title, body (absolute time only, D9.12)
// view — domain to IPC read model
pub struct ProviderState<'a> { pub provider: ProviderKey, pub usage: &'a Usage, pub refresh: RefreshView }
pub fn snapshot(states: &[ProviderState<'_>], now: UnixMillis, settings: &Settings) -> Snapshot;
pub fn same_content(a: &Snapshot, b: &Snapshot) -> bool;                              // ignores generated_at
```

**Acceptance tests.**
- `derive`: table test over every combination of (`attempted`, `last_good` age, failure
  presence/kind) → the exact `Usage` of D4.3/D7.7; property: total (never panics), never
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
  "as of HH:MM"; the notification body has no relative time.
- `reset_marks`: a fixed `resets_at` gives one mark per window; a `resets_at` that slides by
  seconds on every sample still gives one mark per window length; at most 64. `gap_for`
  values for each range's bucket.
- `severity` for a balance: `Critical` when not usable, else `Ok`.
- `should_store`/`downsample`: bounds (`len ≤ max`), first and last kept, extremes kept,
  monotone timestamps; `range_bounds` for each range.
- `Settings`: defaults (80/95; notifications master on and both levels on; system theme;
  auto hour cycle; close to tray on; **autostart on** (D1a); shortcuts on; live check off;
  `first_run_seen` false; no hidden provider); `validated` clamps to 50..=100 and enforces
  warn < critical;
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
/// inotify on the parent directory, filtered to the file name, coalesced (trailing edge).
/// Never fails: when the directory is missing or the watch is lost it re-arms itself at
/// 1, 2, 4 … 15 min (D7.2) and signals once when armed. A queue overflow is a signal.
pub struct WatchOptions { pub coalesce: Duration, pub rearm_first: Duration, pub rearm_max: Duration }   // Default: 1.5 s, 1 min, 15 min
pub fn watch_file(path: &Path, id: SourceId, tx: mpsc::Sender<SourceId>, opts: WatchOptions) -> WatchGuard;
// sources::claude_log
pub struct ClaudeGatewayLogSource;
impl ClaudeGatewayLogSource { pub fn new(path: PathBuf, facts: Arc<FactsCell>) -> Self; }
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
- `watch_file`: 200 appends in 100 ms produce one signal; a rename-rotation produces a
  signal; delete-and-recreate produces a signal; writes to a sibling file produce none; no
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
- [ ] No polling loop, no `sleep` in a loop; only inotify + coalescing timer.
- [ ] Rotation logic proven by the property test, including the inode check.
- [ ] No `String::from_utf8_lossy`/logging of line bytes anywhere.
- [ ] The only file names in this code are derived from the injected path.

---

## T04 — Subprocess sources: Codex app-server and Claude live check (wave 1)

**Goal.** The single, locked-down process spawner and the two sources that use it.

**Owns.** `dashboard/crates/usage-io/src/{proc.rs,sources/codex.rs,sources/claude_live.rs}`,
`dashboard/crates/usage-io/tests/{proc_*.rs,codex_*.rs,claude_live_*.rs}`, test stub
programs under `dashboard/crates/usage-io/tests/stubs/` (shell scripts).

**Exposes.**
```rust
// proc (the only module allowed to use std/tokio process APIs)
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum Program { Codex, Claude }
pub struct ProcSpec { pub program: Program, pub args: &'static [&'static str],
                      pub stdin: &'static [u8], pub deadline: Duration }
pub enum Flow { Continue, Done }      // Done: close stdin and wait for exit
pub struct ProcRunner;
impl ProcRunner {
    pub fn system(workdir: PathBuf) -> Self;                                   // resolves "codex"/"claude" on PATH
    pub fn with_programs(workdir: PathBuf, codex: PathBuf, claude: PathBuf) -> Self;   // feature "test-support"
    /// Spawn under the rules of D8.6, write `stdin`, call `on_line` per stdout line.
    pub async fn run(&self, spec: &ProcSpec, on_line: &mut (dyn FnMut(&[u8]) -> Flow + Send)) -> Result<(), Unavailable>;
}
pub const ENV_ALLOW: &[&str];   // D8.6 list
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
  its environment; assert only `ENV_ALLOW` names are present and no canary. `ENV_ALLOW`
  equals the D8.6 list exactly (snapshot test).
- Readiness `Unknown` → `Unavailable(Timeout)` and no process started.
- stderr of the child containing the canary never reaches `diag` capture or the result.
- Working directory of the child is the given workdir. No shell is involved (`ps` of the
  stub shows the program itself as argv0).
- Readiness `NotLoggedIn`/`NotInstalled` → no process started (stub writes a marker file
  when run; assert absent).
- There is no way to construct a `Program` for any other binary (compile-level: the enum).
- **Manual check, recorded in the report** (free, no prompt, allowed by D3.1): one real
  `codex app-server` run through `ProcRunner::system` with the scrubbed environment and the
  empty working directory returns a reading on this machine. If it does not, report which
  variable was missing; do not widen `ENV_ALLOW` without the lead.

**Security rules.** D8.6 in full, D8.3 (`account/read` never sent: test greps the request
constant), D6.2 (live check only on manual trigger), D8.4.

**Depends on.** T00 (uses T01's `parse::codex` and `parse::claude_event`).

**Review checklist.**
- [ ] `env_clear()` then allow-list; own process group; group kill on deadline and on drop.
- [ ] stderr is `Stdio::null()`; stdout bounded per line and in total.
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
/// Environment first, else the env file (read into a zeroizing buffer, parsed by embed::parse_env).
pub fn deepseek_key(sources: &KeySources) -> Option<Secret<String>>;
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
  empty value → `None`; the file buffer is zeroized (test through a wrapper type that
  records drop-zeroization).
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
pub fn ensure_private_dir(path: &Path) -> std::io::Result<()>;                 // create 0700, tighten if looser
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()>; // tmp 0600 + fsync + rename + dir fsync
// store
pub struct SqliteStore;
pub struct StoreStats { pub bytes: u64, pub samples: u64 }
impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError>;       // pragmas, migrate, corrupt → rename aside + recreate
    pub fn stats(&self) -> Result<StoreStats, StoreError>;
    pub fn meta_get(&self, key: MetaKey) -> Result<Option<i64>, StoreError>;
    pub fn meta_set(&self, key: MetaKey, value: i64) -> Result<(), StoreError>;
}
// MetaKey is defined by T00: BackfillDone, LastPruneAt, WindowWidth, WindowHeight
impl HistoryStore for SqliteStore   // record/record_all apply history::should_store; series aggregates in SQL (D7.5)
pub const SCHEMA_VERSION: i64 = 1;
// settings_file
pub struct JsonSettingsStore;
impl JsonSettingsStore { pub fn new(path: PathBuf) -> Self; }
impl SettingsStore for JsonSettingsStore   // load: missing/invalid → defaults (invalid file renamed *.invalid); save: write_private_atomic
```
Schema, pragmas, retention and migration rules: D7.5. `series` returns `SeriesData`: points
with `lo/hi` (equal to `value` for raw samples), never more than `max_points`, aggregated by
SQL `GROUP BY` so the raw rows of a range are never loaded; the bucket width; and the reset
marks (`history::reset_marks` over the distinct `resets_at_s` of raw samples in range).
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
- Memory: `series` over 100 000 raw samples returns ≤ `max_points` points without
  materialising the rows (assert on the statement: one aggregated query, no per-row loop).
- Reset marks: returned for raw ranges, empty when the range is served from rollups.
- Retention with a fake `now`: raw older than 45 d gone, rolled into hours first; rollups
  older than 400 d gone; `series` over a quarter merges rollups and raw without overlap.
- Property: for arbitrary sample sequences, `series(..., max)` has `len ≤ max`, sorted,
  within range, and contains the global min and max of the range.
- Migration: a v0 (empty) file migrates to v1; a file with `user_version = 99` is not
  written (returns `Unavailable` on `record`, still serves `series`); a file of random bytes
  is renamed to `history.db.corrupt-*` and a fresh store works.
- Crash safety: kill a child process mid-transaction (test helper binary), reopen, database
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

## T07 — UI foundation and design system (wave 1, UI-design pass)

**Skills.** Load `frontend-design` for the visual direction within D9.1/D9.7, then
`impeccable` for the system and primitives. Tokens and wireframes in D9 are the contract;
propose deviations to the lead, do not just apply them.

**Goal.** Tokens, primitives, formatting, the IPC seam, stores, router and app shell, so the
screen tasks only compose.

**Owns.** `dashboard/src/lib/design/**` (tokens.css, base.css, fonts), `dashboard/src/lib/
components/primitives/**`, `dashboard/src/lib/format/**`, `dashboard/src/lib/copy.ts`,
`dashboard/src/lib/ipc/{client.ts,tauri.ts,mock.ts,fixtures.ts}`,
`dashboard/src/lib/stores/**` (rune files are `*.svelte.ts`: `usage.svelte.ts`,
`settings.svelte.ts`, `clock.svelte.ts`, `announce.svelte.ts`),
`dashboard/src/lib/router.svelte.ts`, `dashboard/src/lib/boot.ts` (the theme-before-paint
script, D9.7), `dashboard/src/App.svelte`, `dashboard/src/main.ts`, `dashboard/index.html`,
their unit/component tests, `dashboard/e2e/screens/_harness/**` (Playwright helpers: theme/
width matrix, axe runner, no-network guard, CSP-violation guard) and
`dashboard/e2e/screens/primitives.spec.ts` with its baselines.

**Exposes.**
```ts
// lib/ipc/client.ts — components and stores import ONLY this, never @tauri-apps/api
export type Result<T> = { ok: true; value: T } | { ok: false; error: CommandError };
export interface IpcClient {
  getSnapshot(): Promise<Snapshot>;
  refresh(target: RefreshTarget): Promise<Result<void>>;
  getHistory(query: HistoryQuery): Promise<Result<HistorySeries>>;
  getSettings(): Promise<Settings>;
  updateSettings(patch: SettingsPatch): Promise<Result<Settings>>;
  getDiagnostics(): Promise<Diagnostics>;
  onSnapshot(cb: (s: Snapshot) => void): Promise<() => void>;
  onNavigate(cb: (r: Route) => void): Promise<() => void>;
}
export function createTauriClient(): IpcClient;                  // lib/ipc/tauri.ts
export function createMockClient(script?: MockScript): MockIpcClient;   // lib/ipc/mock.ts (push snapshots, fail commands)
export const fixtures: Record<FixtureName, Snapshot>;            // lib/ipc/fixtures.ts: one per state of D9.5 + mixed

// lib/stores
export const usage:    { readonly snapshot: Snapshot | null; readonly refreshing: boolean; refresh(t?: RefreshTarget): Promise<void> };
export const settings: { readonly value: Settings; update(p: SettingsPatch): Promise<Result<Settings>> };
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
Primitives (each with documented props, all states, keyboard and ARIA complete):
`Meter` (bar + thresholds + hatch for critical + `role="meter"` + the full level-1 text as
`aria-valuetext`), `Detail` (the level-1 disclosure of D9.1: opens on hover **and** focus,
hoverable, `Esc` closes without moving focus, never covers its trigger), `StatusGlyph`,
`Card`, `Button`, `IconButton`, `CopyCommand`, `Countdown` (re-renders from `clock`),
`Freshness`, `Skeleton`, `RadioGroup`, `Switch`, `Slider`, `NumberField`, `Dialog`, `Icon`
(inline SVG), `LiveRegion`, `ViewHeader`. No primitive nests one interactive element in
another.
`App.svelte` mounts the header, the router outlet with the three screen files (owned by
T09/T10), the single live region (fed by `announce`), global shortcuts (off when
`settings.shortcuts` is false or a text field has focus). `main.ts` suppresses the web
view's context menu in production builds (D8.8). The live region announces level changes by
comparing consecutive snapshots: a meter whose `severity` changed, or a provider whose state
changed, produces one sentence; a changed number alone produces none.

**Acceptance tests.**
- `format` passes every vector in `format-vectors.json`; property tests with fast-check are
  not required, the vectors are.
- Contrast test computed from `tokens.css` for both themes meets D9.10 (text ≥ 4.5,
  primary ≥ 7, large numerals ≥ 4.5, control boundaries, focus ring, bar fill on track and
  status glyphs ≥ 3).
- Palette test: the token file defines the neutrals, one accent, warn, critical and unknown
  and nothing else; `ok` is an alias of `accent` (D9.7).
- Every primitive: Testing Library tests for each state and keyboard behaviour; **zero axe
  violations**; screenshot per state x theme (`primitives.spec.ts`).
- `Detail`: opens on focus and on hover with the same content; stays while hovered; `Esc`
  closes and focus stays on the trigger; its text equals the trigger's `aria-valuetext`
  detail; it does not overlap the trigger at 320 px or at 200 % zoom.
- Target size: every interactive primitive measures ≥ 32 x 32 px hit area (Playwright
  bounding boxes).
- `announce`/live region: a snapshot sequence in which a meter goes ok → warn announces one
  sentence; twenty snapshots that change only numbers announce nothing; three changes within
  2 s give one merged message; the same message is not repeated within 60 s; a background
  refresh does not announce "Updated".
- Stores: with the mock client, a pushed snapshot updates `usage`; a snapshot event that
  arrives before `getSnapshot` resolves is not overwritten by the older reply; `refreshing`
  follows `in_flight`; `Busy` is a no-op; `clock` stops ticking when `document.hidden`;
  router back/escape; the fragment `#view=settings&theme=dark` opens Settings with
  `data-theme="dark"` set before first paint (no flash test); a fragment with unknown keys
  or values falls back to the overview and the system theme.
- Stylelint: no raw colour, font-size or spacing outside the token files. ESLint security
  rules pass (raw-HTML sinks, web storage, `style=` attributes). Bundle: fonts ≤ 120 KB.
- The harness serves the build with the CSP of `tauri.conf.json`; a `style=""` attribute or
  an inlined `data:` asset in a test page makes the CSP guard fail (shown once in the report).
- Reduced motion and forced-colors screenshots of the primitives.

**Security rules.** D8.8 (no inline styles as strings, no `{@html}`, no `innerHTML`; CSSOM
custom properties only), D8.7 (no runtime request: fonts and icons bundled), D7.4 (types
only from the generated bindings; no hand-written IPC types).

**Depends on.** T00.

**Review checklist.**
- [ ] Components import `IpcClient`, never `@tauri-apps/api`.
- [ ] Tokens match D9.7 or the deviation was approved; no magic numbers in components.
- [ ] Status is never colour-only (glyph + word + pattern present in `Meter`).
- [ ] `Meter` and `Countdown` have correct `aria-valuetext`; live region is single, polite
      and not spammy (D9.10).
- [ ] Minimal (D9.1): no shadows on cards, no decorative icons, one accent; a healthy
      screen is neutral plus one blue.
- [ ] Level 1 is reachable and dismissible by keyboard and identical for hover and focus.
- [ ] Rune files are `*.svelte.ts`; nothing uses web storage; nothing sets a `style` attribute.
- [ ] No timer runs while the document is hidden.
- [ ] Fixtures cover every `UsageView` state, every `Unavailable` and `Setup` value.

---

## T08 — Usage service: the one loop (wave 2)

**Goal.** Wire sources, store, settings, alerts and the snapshot channel into the single
event-driven task of D7.1–D7.2.

**Owns.** `dashboard/crates/usage-io/src/{registry.rs,service.rs,timer.rs}`,
`dashboard/crates/usage-io/tests/{service_*.rs,timer_*.rs}`.

**Exposes.** Exactly D7.1 (`ServiceDeps`, `ServiceHandle`, `spawn`), plus:
```rust
// registry
pub struct RegistryInputs { pub usage_log: Option<PathBuf>, pub facts: Arc<FactsCell>, pub clock: Arc<dyn Clock>,
                            pub runner: Arc<ProcRunner>, pub http: Arc<HttpClient> }
/// The one list of sources. Adding a provider is one line here. The live-check source is
/// always in the list; whether it may run is the setting, checked by the service at call time.
pub fn build(i: &RegistryInputs) -> Vec<Arc<dyn UsageSource>>;
/// The first-run backfill job, or None when `MetaKey::BackfillDone` is set or there is no log path.
/// The job runs `sources::claude_log::backfill` and then sets the meta key.
pub fn backfill_job(usage_log: Option<PathBuf>, store: Arc<SqliteStore>) -> Option<Backfill>;
// service
pub struct Backoff;   // pure: next_delay(failures, kind, jitter: f64) -> Duration   (D7.2)
pub struct SystemJitter;  impl Jitter for SystemJitter   // `Jitter`, `Backfill`, `ShellFacts` are T00's types
// timer (D7.1): bodies of SystemTimer (timerfd, CLOCK_REALTIME, absolute, cancel-on-set) and FakeTimer
```
`RegistryInputs.live_check` is removed (the setting can change at run time; the registry does
not read it).
Behaviour to implement: D7.1 loop with **one** `WallTimer` wait (never
`tokio::time::sleep` for a deadline); D7.2 in full: signal sources, intervals, visibility,
backoff (`NotConfigured`/`Unsupported` are not failures), one-in-flight, the exact manual
refresh semantics (returns when queued; 10 s floor; `Busy`/`NotEnabled` cases; `live_check`
allowed only when the setting is on and the source's cost is `SpendsQuota`), and the engine
probe schedule (start; before a gated fetch when not `Ready` and older than 60 s, also while
hidden; manual at most once per 60 s; 6-hourly piggy-back) through the injected
`EngineProbe`, writing the result to `FactsCell`; merge of several sources per provider by
greatest `observed_at` (an older reading is dropped; a future `observed_at` is clamped to
now, D4.3); `assess::derive` per provider; `store.record` and `alerts::step` +
`notifier.notify` + `save_alerts` on each new reading, every store and settings call through
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
- Backoff sequence 1, 2, 4 … 60 min with injected jitter; reset on success; `RateLimited`
  starts at 15 min; `NotConfigured` never backs off; manual refresh bypasses it.
- Manual refresh: returns before the fetch completes; `all` twice within 10 s starts each
  source once; `provider` while its source is in flight → `Busy`; `all` never returns `Busy`.
- **Suspend**: the fake clock jumps forward 8 h past a window's reset and two interval
  deadlines; the fake timer fires at once; each due source runs once, not eight times, and
  the window is `elapsed` in the next snapshot.
- **Clock set**: `Wake::ClockChanged` with the clock moved back 1 h and forward 1 h: no
  panic, no negative age, deadlines recomputed, no fetch storm.
- A reading stamped 5 minutes in the future is stored and shown with `observed_at = now`.
- **Probe**: with a `FixedProbe` that first answers `Unknown` and then `Ready` for Codex,
  while hidden: the first gated fetch yields `Unavailable(Timeout)`, the retry re-probes and
  the source is fetched; the probe is called at most once per 60 s under 100 manual
  refreshes; a 6-hour-old `Ready` is re-probed on the next due fetch, with no extra wake.
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
- `live_check` refused when disabled; allowed when enabled; never scheduled by a timer.
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
- A panicking or hanging fake source cannot stall the loop (timeout → `Timeout`, others
  continue).

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
- [ ] The service never matches on `SourceId`/`ProviderKey` to special-case behaviour.
- [ ] Every scheduling rule has a paused-time test; no real sleeps in tests.
- [ ] The idle proof test exists and is meaningful.

---

## T09 — Overview screen and all provider states (wave 2, UI-design pass)

**Skills.** `impeccable` while building; finish with a `better-ui` pass (radii, optical
alignment, hit areas).

**Goal.** The screen of D9.3 with every state of D9.5, composed from T07's primitives.

**Owns.** `dashboard/src/screens/Overview.svelte`, `dashboard/src/lib/components/overview/**`
(`ProviderCard`, `MeterRow`, `BalanceRow`, `CardState` variants for pending / stale-quiet /
stale-failing / elapsed / unavailable / not-configured, `FirstRun`, `UnsupportedLine`),
`dashboard/src/lib/overview/**` (pure helpers: `orderProviders`, `cardSummary`,
`rowDetail`), their tests, `dashboard/e2e/screens/overview.spec.ts` and its baselines.
There is no offline banner (D9.5).

**Consumes.** `usage`, `settings`, `clock`, `router`, `announce` stores; `format`, `copy`; primitives.
No IPC calls except through the stores.

**Exposes.** `orderProviders(views: ProviderView[]): ProviderView[]` (D9.3 ordering, stable),
`cardSummary(view, nowMs, hour): string` (the card header button's accessible name),
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
  header is one button named by `cardSummary`; every meter row is a tab stop; inner controls
  are separate tab stops; no interactive element is nested in another; tab order equals
  reading order (asserted as a recorded focus sequence per fixture).
- **Progressive disclosure**: at rest the row shows no absolute time; focusing the row with
  Tab shows the level-1 detail, hovering shows the same text, `Esc` closes it and focus
  stays; the row's `aria-valuetext` contains the same detail with nothing opened; the
  detail never overlaps its row at 320 px or 200 % zoom.
- **Announcements**: the mixed fixture going ok → warn announces one sentence in the live
  region; twenty number-only updates announce nothing.
- Ordering unit tests (severity, then fixed order; no reorder on equal severity).
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
- 200 % zoom and 320 px width: no clipped text, no horizontal scroll.
- Rapid snapshot updates (20 per second from the mock) cause no layout shift (CLS measured
  in Playwright = 0).

**Security rules.** D8.8 UI rules; only text from `copy.ts`, `format` and typed fields is
rendered; no dynamic HTML.

**Depends on.** T07.

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
export function makeScale(series: HistorySeries, width: number, height: number, pad: Pad): Scale;
export function stepPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // breaks at gaps; gapSeconds is always series.gap_s
export function bandPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // lo..hi
export function ticks(series: HistorySeries, range: HistoryRange, hour: HourCycle): { x: Tick[]; y: Tick[] };
export function summarize(series: HistorySeries, hour: HourCycle): string;                     // the role="img" label
export function nearest(points: HistoryPoint[], t: number): number;                            // index for the crosshair
// lib/detail/history.svelte.ts
export const history: { load(q: HistoryQuery): Promise<void>; readonly state: "idle" | "loading" | "ready" | "empty" | "error"; readonly series: HistorySeries | null };
```

**Acceptance tests.**
- Chart geometry: step path for known points equals an expected path string; gaps split the
  path exactly where two points are further apart than `series.gap_s` (the UI has no gap
  constant of its own); band for rollups; empty series → empty state ("No history yet. It
  builds up as you use the provider."); single point renders a dot; values above 100 %
  extend the axis; reset hairlines from `series.resets`, and the summary omits the reset
  count when `resets` is empty (month, quarter).
- Chart a11y: `role="img"` label from `summarize`; hidden data table has one row per point
  (capped at 600); Left/Right move the crosshair and announce the value; pointer hover shows
  the same readout; axe clean.
- Range switch is a radio group with arrow keys; changing range/meter issues exactly one
  `getHistory` and cancels a stale response (out-of-order responses do not overwrite).
- Detail states: provider `unavailable`/`not_configured`/`unsupported` show the state block
  and no empty chart frame; `stale` shows the freshness note.
- Live check button exists only when the setting is on and `manual === "spends_quota"`;
  it states the cost; `NotEnabled`/`Busy` handled.
- Detail view is level 2 of D9.1: every window in full (number, countdown and absolute time
  side by side, status), without needing hover.
- Settings covers every field of D7.4 `Settings` except `first_run_seen`: the notification
  master switch with the two level switches disabled (values kept) while it is off; "Start
  at login" **on by default**, rolling back with a reason on an `io` error; the single-key
  shortcuts switch; "About and diagnostics" showing the name Flashboard and the version.
- Settings: every control reads and writes through `settings.update`; thresholds validate
  on blur (warn < critical, 50..100) with a specific message; sliders and number fields stay
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

**Security rules.** D8.8 UI rules. Diagnostics render only the typed fields of
`Diagnostics` (no paths, no raw errors).

**Depends on.** T07.

**Review checklist.**
- [ ] Step chart, no interpolation across gaps, thresholds labelled in text.
- [ ] No chart dependency added; SVG only; colours from tokens.
- [ ] Chart is usable by keyboard and described for screen readers.
- [ ] Settings apply immediately and can never store an invalid threshold pair.
- [ ] Live-check consent dialog wording states the cost plainly.

---

## T11 — Tauri shell: commands, events, tray, notifications, window lifecycle (wave 3)

**Goal.** The thin shell of D5.2: wiring only, no business logic.

**Owns.** `dashboard/src-tauri/src/**` (`main.rs`, `commands.rs`, `events.rs`, `tray.rs`,
`tray_icon.rs`, `notifier.rs`, `window.rs`, `autostart.rs`, `hardening.rs`,
`localtime.rs`, `bus.rs`, `cli.rs`), `dashboard/src-tauri/tests/**`.

**Exposes / implements.**
```rust
// commands.rs — exactly these six, names fixed by the capability file
#[tauri::command] fn get_snapshot(svc: State<'_, ServiceHandle>) -> Snapshot;
#[tauri::command] async fn refresh(svc: State<'_, ServiceHandle>, target: RefreshTarget) -> Result<(), CommandError>;
#[tauri::command] async fn get_history(svc: State<'_, ServiceHandle>, query: HistoryQuery) -> Result<HistorySeries, CommandError>;
#[tauri::command] fn get_settings(svc: State<'_, ServiceHandle>) -> Settings;
#[tauri::command] async fn update_settings(app: AppHandle, svc: State<'_, ServiceHandle>, patch: SettingsPatch) -> Result<Settings, CommandError>;
#[tauri::command] async fn get_diagnostics(app: AppHandle, svc: State<'_, ServiceHandle>) -> Result<Diagnostics, CommandError>;   // ShellFacts: version + tray host asked now
// events.rs
pub const EVENT_SNAPSHOT: &str = "snapshot"; pub const EVENT_NAVIGATE: &str = "navigate";
pub fn forward_snapshots(app: AppHandle, rx: watch::Receiver<Snapshot>);   // emits only to the "main" window, only while it exists
// tray_icon.rs (pure)
pub struct IconSpec { pub used_step: u8 /* 0..=20, 5 % steps */, pub level: Severity }
pub fn render(spec: IconSpec, size: u32) -> Vec<u8>;                        // RGBA, D9.11 shapes
// tray.rs
pub fn install(app: &AppHandle) -> Result<TrayState, TrayError>;            // TrayError is a field-less enum; id and title "Flashboard"
/// Rows from core::text::tray_rows (each opens its provider), icon from IconSpec. Rebuilds
/// only when (rows, IconSpec) differ from the last build (D7.7).
pub fn update(app: &AppHandle, snapshot: &Snapshot, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle);
// bus.rs — the only module that uses zbus
pub trait TrayHost: Send + Sync { fn present(&self) -> bool; }              // injected, so window tests need no bus
pub struct SessionBusTrayHost; impl TrayHost for SessionBusTrayHost        // org.kde.StatusNotifierWatcher owned and IsStatusNotifierHostRegistered; asked each time, never cached
// notifier.rs — the only module that uses notify_rust
pub struct DesktopNotifier; impl Notifier for DesktopNotifier              // title/body from core::text::notification; urgency by level; sends on a blocking thread, never blocks the caller
// window.rs
pub fn open(app: &AppHandle, route: Route);                                 // create (URL fragment: view, provider, theme; incognito) or show + `navigate` + focus request
pub fn close_requested(app: &AppHandle);                                    // tray host present now and close_to_tray: hide, destroy after 30 s; else quit
pub fn remember_size(app: &AppHandle, w: u32, h: u32);                      // debounced into the store's meta (WindowWidth/WindowHeight)
// autostart.rs
pub fn desktop_entry(exec: &Path) -> String;                                // pure; Name=Flashboard, Exec=<exec> --hidden
/// Makes ~/.config/autostart/dev.jev.flashboard.desktop match `enabled` (atomic, 0600; rewrites when Exec differs).
pub fn reconcile(enabled: bool, exec: &Path, config_home: &Path) -> std::io::Result<()>;
// hardening.rs
pub fn apply();                                                             // umask 077, RLIMIT_CORE 0, PR_SET_DUMPABLE 0, panic hook (base name and line only)
// localtime.rs
pub struct SystemLocalTime; impl LocalTime for SystemLocalTime             // chrono Local
```
`main.rs` order: `hardening::apply()` (before any thread) → builder with the
single-instance plugin registered first (a second launch calls `window::open` unless its
arguments contain `--hidden`) → in `setup` (which runs after plugin initialisation, so a
second process never gets here): build `ServiceDeps` (engine_bridge paths, `SqliteStore`,
`JsonSettingsStore`, `registry::build`, `registry::backfill_job`, `watch_file`,
`SystemProbe`, `SystemTimer`, `SystemJitter`, `DesktopNotifier`) → `spawn` →
`autostart::reconcile` from the loaded settings (release builds only; a failure is
`DiagEvent::AutostartFailed`, never fatal) → tray → window unless `--hidden` → run loop.
`ExitRequested` is prevented only when it carries no exit code; tray Quit awaits
`ServiceHandle::shutdown()` and then exits. Service visibility follows the window
(`set_visible`). Navigation locked to the app origin; new windows denied. The web view is
created in code on open (`incognito(true)`, size from the store's meta) and destroyed 30 s
after close (D9.13). `update_settings`: after the service accepts the patch, if `autostart`
changed the shell reconciles the entry; on failure it restores the previous value through
the service and returns `CommandError { code: "io" }`. The tray's Notifications checkbox
writes `settings.notifications`; the shell re-renders tray text when settings change
(`subscribe_settings`). `DiagEvent::WindowReady` is emitted when the first `get_snapshot`
of a newly created web view returns.

**Acceptance tests.**
- `tray_icon::render`: golden-image tests for representative specs (0, 50, 80, 95, 100 % x
  levels); property: every pixel buffer has length `size*size*4`, alpha-premultiplied
  correctly, deterministic.
- `autostart::desktop_entry` golden text (validated with `desktop-file-validate`);
  `reconcile` in a temp config home: on creates the file 0600, off removes it, a changed
  `exec` rewrites it, repeated calls change nothing, an unwritable directory returns an
  error and leaves no partial file.
- `update_settings` with a failing reconcile returns `io` and the stored setting is the old
  value.
- `hardening::apply`: after the call, a created file is 0600 and `/proc/self/status` shows
  the process is not dumpable (test in a child process); panic hook output contains a base
  name and a line, no `/`, and not the payload (child process with a canary payload).
- `DesktopNotifier` with an injected sender that blocks: `notify` returns immediately.
- Tray: `update` twice with snapshots that differ only in `RefreshView` rebuilds nothing;
  a changed row rebuilds once; a row's menu id maps back to its provider.
- Second launch: without `--hidden` opens the window; with `--hidden` does nothing.
- Command layer with `tauri::test` mock runtime: each command returns the handle's value;
  invalid arguments (unknown provider, malformed meter id, out-of-range thresholds) return
  `invalid_argument`; payload JSON equals the bindings' shape.
- Snapshot tests: capability file, CSP, plugin list, absence of the `devtools` feature,
  `withGlobalTauri: false`, exactly six commands registered.
- Window lifecycle with the mock runtime and a fake `TrayHost`: close → hidden → destroyed
  after 30 s (paused time); open recreates with the route and theme in the fragment; open
  on an existing window emits `navigate`; `--hidden` creates no window; no tray host at
  close time → close quits; `--hidden` with no tray host stays resident; a host that appears
  after start is seen at the next close (nothing cached).
- Manual check list (recorded in the report), on this machine: tray icon, title and menu in
  the quickshell host, a menu row opens that provider; a warn and a critical notification
  appear once each with the right urgency; a second launch opens the window; the size is
  remembered; light and dark follow the OS in the real WebKitGTK view and the Theme setting
  overrides it; **log out and in: the app is running hidden, with Claude and Codex found**
  (autostart and its `PATH`); with `PR_SET_DUMPABLE=0` the window, clipboard copy and theme
  still work (else fall back per D8.8 and report); after closing the window for 30 s no
  WebKit helper process remains and no directory was created under `~/.local/share` or
  `~/.cache` for the app; suspend past a window's reset and resume: the tray is correct
  within seconds; the app runs beside a live `jev-claude` session with no effect on it.

**Security rules.** D8.8 in full; D8.4 (no logging except `diag`; panic hook); D7.3 (no
extra command, no event from UI to Rust); notifications and tray text only from
`core::text`; the shell reads no file itself except through `fsutil`/stores.

**Depends on.** T08 (and the event/command names used by T07's `tauri.ts`).

**Review checklist.**
- [ ] No logic in the shell that belongs in core/io (no formatting, no thresholds, no parsing).
- [ ] Exactly six commands; capability and CSP snapshots unchanged from T00.
- [ ] Web view destroyed when closed; service marked hidden; no timer in the shell except
      the 30 s destroy and the size debounce.
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
  process APIs (`proc.rs`), `zbus` (`bus.rs`), `notify_rust` (`notifier.rs`), `jev_router::`
  paths; forbidden file names; forbidden macros; no `String` in error or core type files),
  the **IPC audit** (key allow-list, forbidden key pattern, string-leaf vocabulary, over
  proptest-generated values of every IPC type and over the generated `.ts` files), the
  **end-to-end canary** (service + real adapters over stubs + real store in a temp state
  dir; search snapshots, all files' raw bytes, `diag` capture), the **no-network** Playwright
  test (any request other than app origin/IPC fails; CSP violation events fail).
- Fuzz targets for the seven entry points of D11.2 with seed corpora from the fixtures, and
  the stable byte-level proptests that mirror them.
- **Real-binary smoke test** (stage 10 of D11.4, no driver): start the `dist` binary in the
  current graphical session with `HOME` and every `XDG_*_HOME` and `XDG_RUNTIME_DIR`-relative
  path it writes pointing at a temp tree containing a synthetic, canary-laced `usage.jsonl`,
  and a `PATH` that contains only stub `codex` and `claude` programs (no real CLI is ever
  run; the autostart entry lands in the temp config home). Assert from stderr `diag` lines
  and the temp tree: `Started`; a successful Claude-log and Codex fetch; `WindowReady` within
  the D10 budget; a new sample within 3 s of an appended row; rows in `history.db`; file
  modes; no canary anywhere in the tree or on stderr; no WebKit data directory; nothing
  written outside the tree (compare a listing of the real state and config directories
  before and after); clean exit on SIGTERM with no child left.
- WebDriver e2e (stage 13, `tauri-driver` + `WebKitWebDriver` when present): the same
  launch; assert the overview shows the fixture numbers; open detail, settings; change a
  threshold; verify `settings.json`. Without the driver the stage prints NOT RUN (needs
  root) with install instructions and is the one stage `--release` tolerates (D11.4).
- `measure-idle.sh`: launches the binary hidden for 10 minutes against the temp tree and
  reports CPU time, wakeups (voluntary context switches) and RSS, each **summed over the
  process tree and all threads** (`/proc/<pid>/task/*/status`), and the binary size, against
  the D11.3 budgets.
- `ci.sh`: all thirteen stages of D11.4, `--bootstrap` (no root: cargo tools, pinned
  nightly, `tauri-driver`, Playwright Chromium), `--release`, a final table with
  PASS/FAIL/SKIPPED/NOT RUN per stage and total time; non-zero exit on any FAIL (and on
  SKIPPED with `--release`).
- Accessibility gate: stage 6 and stage 8 fail on any axe violation, on the contrast test,
  and on the keyboard-path and live-region tests of D9.10; a table in the report maps every
  bullet of D9.10 to the test (or manual step) that checks it.

**Acceptance.** `scripts/ci.sh` green on this machine; a deliberate violation of each rule
(a `println!`, a second `expose_secret`, an extra IPC field named `email`, a string leaf, an
extra capability permission, a remote `<img>`, a `style=""` attribute, a `localStorage`
call, a low-contrast token, an unnamed icon button) makes the matching stage fail
(demonstrated in the report, not committed); coverage thresholds met or the gap reported per
file.

**Security rules.** All of D8. This task may not weaken a rule to make a test pass; a rule
that cannot be met is escalated to the lead.

**Depends on.** T01–T11; final run after T13 is merged (section 1).

**Review checklist.**
- [ ] Each row of D8.10 and each bullet of D9.10 maps to a named, passing test or a named
      manual step (tables in the report).
- [ ] Mutation check done: each proof fails when its rule is broken.
- [ ] `ci.sh` has no silent skip; engine stage first and unchanged.
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
- D12 build and install flow; `install.sh [dest]` and `--uninstall`; desktop entry
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
  every screen and state, what Orca must say, in order. The manual pass against it, with
  notes, on the installed app in the real WebKitGTK view; and a keyboard-only walk-through.
  Orca is not installed (D17 Q2): if it still is not when this task runs, deliver the script
  and the keyboard walk-through, and report the pass as the one open item of v1.
- README: install, autostart (on by default, how to turn it off, what file it writes), the
  `PATH` note, optional Hyprland window rule, privacy summary with the honest limits of D8
  (including what the engine probe reads), troubleshooting.

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
- [ ] The screen-reader script exists and the pass is done or explicitly reported open.
- [ ] Nothing says "Jev Dashboard" or `dev.jev.dashboard`.

---

## 2. Lead review protocol (for every task)

1. Run `scripts/ci.sh`. Read the diff in full; check file ownership against this document.
2. Walk the task's checklist; for security rules, open the named test and confirm it would
   fail if the rule were broken.
3. Search the diff for: new dependencies, `allow(` attributes, `String` in error or IPC
   types, `unwrap`/`expect`, new file names read from disk, new network hosts, new commands
   or permissions, timers and loops.
4. For UI tasks: review every baseline screenshot in both themes at 360 and 1200 px, tab
   through the screen, read the copy aloud; check the four words of D1a (minimal, detailed,
   beautiful, accessible) against D9.1 and every bullet of D9.10.
5. Reject with specific, reproducible findings; accept only when D11.5 is fully met.
6. Repeat the review round until a full round finds nothing (D1a). Only then is the task done.
