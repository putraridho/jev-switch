# Jev Dashboard: work breakdown

Read `DESIGN.md` first. It is normative; this file says who builds which part of it.
Section numbers below ("D4.2") refer to `DESIGN.md`.

## 0. Rules for every worker

1. **Ownership is exclusive.** Edit only the files your task owns. Every file in the tree is
   created by T00 (as a compiling stub with its final public signatures) and then owned by
   exactly one later task. If you need a change in a file you do not own, or in a **frozen**
   file (list below), stop and send the lead a contract change request; do not edit it.
2. **Frozen after T00** (lead approval to change): every `Cargo.toml`, `Cargo.lock`,
   `deny.toml`, every `lib.rs`/`mod.rs`, `src/embed.rs`, `dashboard/package.json`,
   `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`, lint configs,
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
7. Done means D11.5. Report with: files changed, tests added (names), how each security rule
   of the task is proven, anything you could not verify.
8. The engine (`src/`, root tests, `scripts/install.sh`) must stay green and untouched
   except `src/embed.rs` and one `pub mod embed;` line (T00).

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
the neutral stubs).

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
- Root: the `[workspace]`, `[workspace.lints]`, `[profile.dist]` tables in `Cargo.toml`
  (nothing else in that file changes), `Cargo.lock`, `deny.toml`, `src/embed.rs`, one line
  `pub mod embed;` in `src/lib.rs`, `scripts/ci.sh` (stages 1–7 and 9 of D11.4 working;
  the rest present and reporting SKIPPED with a reason), `.gitignore` additions.
- `dashboard/crates/usage-core/**`: manifests, `lib.rs`, the frozen type files **complete
  with tests** (`time quantity label provider meter usage port secret ipc`), and stubs for
  `parse/{claude_log,claude_event,codex,deepseek}.rs`, `assess.rs alerts.rs summary.rs
  history.rs text.rs settings.rs view.rs` (types complete, functions returning a neutral
  value: `None`, `Err(Unavailable::Internal)`, an empty vec; no `todo!`, no lint allows).
- `dashboard/crates/usage-io/**`: manifests, `lib.rs`, **complete** `engine_bridge.rs`,
  `clock.rs`, `diag.rs`; stubs for `fsutil tail watch proc http secrets store settings_file
  registry service` and `sources/{claude_log,claude_live,codex,deepseek,placeholder}.rs`
  (`placeholder.rs` complete).
- `dashboard/src-tauri/**`: `Cargo.toml`, `build.rs` (AppManifest commands, D8.8),
  `tauri.conf.json` (CSP, security flags, identifier, no bundle targets),
  `capabilities/main.json`, a `main.rs` that builds and shows an empty window, stub modules
  `commands events tray tray_icon notifier window autostart hardening localtime`.
- `dashboard/` UI: `package.json` (exact versions, scripts `dev build check lint test
  test:screens tauri`), `pnpm-lock.yaml`, `.npmrc`, `tsconfig.json`, `vite.config.ts`,
  ESLint/stylelint/Prettier configs with the rules of D8.10 and D9.7, `index.html`,
  `src/main.ts`, stub `src/App.svelte`, stub screens
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
  pub struct SettingsPatch { /* every field Option<_> */ }              // IPC (Deserialize)
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
  // io::engine_bridge
  pub enum Readiness { Ready, NotInstalled, NotLoggedIn, NotConfigured, Unknown }
  pub struct EngineFacts { /* private */ }
  impl EngineFacts {
      pub async fn probe() -> Self;                                     // embed::probe_all(), mapped
      pub fn from_availability(a: &[embed::Availability]) -> Self;      // pure, tested
      pub fn readiness(&self, p: ProviderKey) -> Readiness;
      pub fn plan(&self, p: ProviderKey) -> Option<Label>;              // Label::parse of the engine's plan, else None
  }
  pub struct FactsCell(/* RwLock<EngineFacts> */);                      // shared read handle
  impl FactsCell { pub fn new(f: EngineFacts) -> Arc<Self>; pub fn get(&self) -> EngineFacts; pub fn set(&self, f: EngineFacts); }
  pub fn usage_log_path() -> Option<PathBuf>;                           // embed::path_from_env("JEV_USAGE_LOG","usage.jsonl")
  pub fn dashboard_dir() -> Option<PathBuf>;                            // <state_dir>/dashboard
  pub fn env_file() -> Option<PathBuf>;                                 // embed::env_file_path(...)
  // io::clock
  pub struct SystemClock;  impl Clock for SystemClock
  pub struct FakeClock;    impl FakeClock { pub fn at(ms: u64) -> Arc<Self>; pub fn advance(&self, d: Duration); pub fn set(&self, ms: u64); }   // feature "test-support"
  // io::diag
  pub enum DiagEvent { SourceOk { source: SourceId, took_ms: u32 }, SourceFailed { source: SourceId, reason: Unavailable },
                       StoreFailed { error: StoreError }, LogRotated, WatchLost, Started, Stopping }
  pub fn emit(e: DiagEvent);                                            // stderr, one line, numbers and enum names only
  pub fn capture() -> CaptureGuard;                                     // feature "test-support": collect instead of print
  ```
- `usage-io` has a Cargo feature `test-support` (FakeClock, diag capture, proc/http test
  constructors) enabled only by dev-dependencies and tests.
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
  PartialEq`; a compile-time check that `Usage`, `Reading`, `SourceError` contain no
  `String` field (they are built only from the frozen types).
- `EngineFacts::from_availability` table test over every engine `State`.
- `scripts/ci.sh` runs and prints the stage table.

**Security rules.** D8.4 (no string in errors, `diag` typed), D8.5, D8.8 (capabilities, CSP,
plugin list exactly as designed), D8.9 (exact pins, lockfiles, `deny.toml`, npm script
blocking and release-age rule), fixtures contain no real data.

**Depends on.** nothing.

**Review checklist (lead).**
- [ ] Root `Cargo.toml` diff is only the three tables; engine build and tests identical.
- [ ] `default-members = ["."]`; `resolver = "2"`; `profile.dist` separate from `release`.
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
Mapping rules: D6.1, D6.3, D6.4, D3.1, D3.2. Details: utilization and reset are strings in
the log and numbers in the event; a window with a utilization but an unparsable reset keeps
`resets_at: None`; statuses `allowed` → `Allowed`, `allowed_warning` → `Warning`,
`rejected` → `Rejected`, anything else → `Unknown`; a window name that fails `Label::parse`
drops that window; more than 8 windows keeps the first 8 in key order; duplicate windows keep
the first.

**Acceptance tests.**
- Each parser against its recorded/synthetic fixtures with exact expected `Meters`
  (`rate_limits_read.response.json` → one `Monthly` window, 400 bp, reset 1797439066,
  plan `free`; notifications and other ids → `None`).
- Paid-plan shape (synthetic from the schema): 300 and 10080 minutes → `FiveHour`, `SevenDay`.
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
pub fn should_store(last: Option<&StoredSample>, candidate: &StoredSample) -> bool;   // D7.5 write rule
pub fn downsample(points: Vec<SamplePoint>, max: u16) -> Vec<SamplePoint>;           // keeps first, last, extremes per bucket
pub fn range_bounds(range: HistoryRange, now: UnixSeconds) -> (UnixSeconds, UnixSeconds);
pub const RAW_RETENTION: Duration; pub const ROLLUP_RETENTION: Duration;              // 45 d, 400 d
// settings
impl Settings { pub fn defaults() -> Self; pub fn validated(self) -> Self; pub fn apply(&self, p: SettingsPatch) -> Result<Self, CommandError>; pub fn thresholds(&self) -> Thresholds; }
// text (D9.9, D9.11, D9.12) — the only place tray/notification strings are built
pub fn percent(p: Percent) -> String;
pub fn window_name(k: &WindowKind) -> String;
pub fn reset_absolute(at: UnixSeconds, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String;
pub fn countdown(at: UnixSeconds, now: UnixSeconds) -> Option<String>;               // None when past
pub fn tray_rows(snapshot: &Snapshot, local: &dyn LocalTime, hour: HourCycle) -> Vec<String>;
pub fn notification(alert: &Alert, now: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> (String, String); // title, body
// view — domain to IPC read model
pub struct ProviderState<'a> { pub provider: ProviderKey, pub usage: &'a Usage, pub refresh: RefreshView }
pub fn snapshot(states: &[ProviderState<'_>], now: UnixMillis, settings: &Settings) -> Snapshot;
pub fn same_content(a: &Snapshot, b: &Snapshot) -> bool;                              // ignores generated_at
```

**Acceptance tests.**
- `derive`: table test over every combination of (`attempted`, `last_good` age, failure
  presence/kind) → the exact `Usage` of D7.7; property: total (never panics), never returns
  `Known` with an old reading, never loses `last_good` on failure.
- `alerts::step` properties over arbitrary reading sequences: at most one alert per
  `(meter, level)` between re-arms; critical suppresses warn in the same step; no alert for
  stale readings, elapsed windows, disabled levels or balance meters; restart safety
  (`step` on a memory round-tripped through the store type gives no duplicates); hysteresis
  of 5 points; re-arm on passed reset.
- `headline`: picks the highest non-elapsed window; ignores hidden providers, `Unsupported`,
  `Pending`; deterministic tie-break by `ProviderKey` order.
- `text` and formatting: every case in `dashboard/fixtures/format-vectors.json` (the same
  file the UI tests use); zone and hour-cycle variants; DST transition days; year rollover.
- `should_store`/`downsample`: bounds (`len ≤ max`), first and last kept, extremes kept,
  monotone timestamps; `range_bounds` for each range.
- `Settings`: defaults (80/95, both notifications on, system theme, close to tray, no
  autostart, live check off); `validated` clamps to 50..=100 and enforces warn < critical;
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
    /// Position at EOF after scanning backwards (≤ max_bytes) for the newest line `accept`s.
    /// Falls back to `<path>.1`. Returns whether a line was accepted.
    pub fn seek_newest(&mut self, max_bytes: u64, accept: &mut dyn FnMut(&[u8]) -> bool) -> Result<bool, Unavailable>;
    /// Feed every complete new line since the last call, following a rotation.
    pub fn read_new(&mut self, on_line: &mut dyn FnMut(&[u8])) -> Result<TailEvent, Unavailable>;
}
/// Whole-file forward scan for the backfill; same line rules, constant memory.
pub fn for_each_line(path: &Path, on_line: &mut dyn FnMut(&[u8])) -> Result<u64, Unavailable>;
pub const BLOCK: usize = 256 * 1024; pub const MAX_LINE: usize = 1024 * 1024;
// watch
pub struct WatchGuard;   // dropping it stops the watch
/// inotify on the parent directory, filtered to the file name, coalesced (trailing edge).
pub fn watch_file(path: &Path, id: SourceId, tx: mpsc::Sender<SourceId>, coalesce: Duration) -> Result<WatchGuard, Unavailable>;
// sources::claude_log
pub struct ClaudeGatewayLogSource;
impl ClaudeGatewayLogSource { pub fn new(path: PathBuf, facts: Arc<FactsCell>) -> Self; }
impl UsageSource for ClaudeGatewayLogSource   // Trigger::FileChanged, Cost::Free, timeout 5 s, runs IO in spawn_blocking
/// First-run history backfill: `<path>.1` then `<path>`, under history::should_store. Returns samples stored.
pub fn backfill(path: &Path, store: &dyn HistoryStore) -> Result<u64, StoreError>;
```
`Reading.observed_at` is the row's `ts_ms`; `plan` is `facts.get().plan(Claude)`. No sample
anywhere → `Err(NotConfigured(UseGateway))`. File missing → the same. Permission or read
error → `Err(Unavailable(Io))`.

**Acceptance tests** (temp dirs, synthetic rows from `fixtures/claude-gateway/`).
- Property: for any split of a file into appended chunks (including splits inside a line and
  inside a multi-byte character) and any rotation point (rename to `.1` + new file, with or
  without unread bytes in the old one), the sequence of lines delivered equals a whole-file
  read; no line twice, none lost, none partial.
- Truncation (size below offset) restarts at 0. A line longer than `MAX_LINE` is skipped and
  the next line is delivered. Memory stays below `BLOCK + MAX_LINE` (assert on buffer capacity).
- `seek_newest` finds the newest sample when the last N rows have no `ratelimit`; honours
  `max_bytes`; falls back to `.1`.
- `watch_file`: 200 appends in 100 ms produce one signal; a rename-rotation produces a
  signal; no signal with no writes for 2 s; dropping the guard stops signals. (Real inotify
  in a temp dir, bounded waits.)
- Source: fixture file → expected `Reading`; `canary.jsonl` → no canary in the result;
  the source never opens any path other than `<path>` and `<path>.1` (test with a sibling
  `gateway.jsonl` whose permissions are 000: the source still succeeds).
- `backfill` on a two-file fixture stores the expected number of samples and is idempotent
  through the store's primary key.

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
`Unavailable(NotInstalled | NotLoggedIn)` **without spawning**. Error mapping: spawn failure
→ `NotInstalled`; deadline → `Timeout`; non-zero exit with no response → `Protocol`; output
over the caps → `Protocol`.
Claude live flags: determine from `claude --help` only (do not run a prompt). The source is
unit-tested with a stub program that replays `tests/fixtures/claude/turn1.jsonl`. A real run
is not part of this task.

**Acceptance tests** (stub programs only).
- Stub replays the recorded Codex session (with notifications interleaved and a 200 ms
  delay) → the expected `Reading`; stdin received by the stub equals the three request
  lines exactly.
- Failure matrix: stub exits 1 immediately; prints garbage; prints a 5 MiB line; never
  answers (deadline with paused time, process group is killed: assert no child remains);
  answers with a JSON-RPC error; closes stdout early.
- **Environment**: parent has `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`, `JEV_API_KEY`,
  `ANTHROPIC_BASE_URL` set to canary values; the stub dumps its environment; assert only
  `ENV_ALLOW` names are present and no canary.
- stderr of the child containing the canary never reaches `diag` capture or the result.
- Working directory of the child is the given workdir. No shell is involved (`ps` of the
  stub shows the program itself as argv0).
- Readiness `NotLoggedIn`/`NotInstalled` → no process started (stub writes a marker file
  when run; assert absent).
- There is no way to construct a `Program` for any other binary (compile-level: the enum).

**Security rules.** D8.6 in full, D8.3 (`account/read` never sent: test greps the request
constant), D6.2 (live check only on manual trigger), D8.4.

**Depends on.** T00 (uses T01's `parse::codex` and `parse::claude_event`).

**Review checklist.**
- [ ] `env_clear()` then allow-list; `kill_on_drop`; process-group kill on deadline.
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
  of every error; `expose_secret` is called in exactly one place (source grep in the test).
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
pub enum MetaKey { BackfillDone, LastPruneAt }
impl HistoryStore for SqliteStore   // record applies history::should_store; series merges raw + rollups and downsamples
pub const SCHEMA_VERSION: i64 = 1;
// settings_file
pub struct JsonSettingsStore;
impl JsonSettingsStore { pub fn new(path: PathBuf) -> Self; }
impl SettingsStore for JsonSettingsStore   // load: missing/invalid → defaults (invalid file renamed *.invalid); save: write_private_atomic
```
Schema, pragmas, retention and migration rules: D7.5. `series` returns points with `lo/hi`
(equal to `value` for raw samples) and never more than `max_points`. All text written to the
database comes from `ProviderKey::as_str()` and `MeterId::key()`; all SQL uses bound
parameters.

**Acceptance tests** (temp dirs).
- Modes: directory 0700; `history.db`, `-wal`, `-shm`, `settings.json` and the atomic temp
  file 0600, also when the process umask is 022 at test time.
- `record` + `series` round-trip; write rule (unchanged value within 15 min not stored;
  changed value stored; heartbeat stored); primary-key idempotence.
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
`dashboard/src/lib/stores/**`, `dashboard/src/lib/router.ts`, `dashboard/src/App.svelte`,
`dashboard/src/main.ts`, `dashboard/index.html`, their unit/component tests, and
`dashboard/e2e/screens/_harness/**` (Playwright setup, theme/width matrix, no-network guard).

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
export const router:   { readonly route: Route; go(r: Route): void; back(): void };
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
```
Primitives (each with documented props, all states, keyboard and ARIA complete):
`Meter` (bar + thresholds + hatch for critical + `role="meter"`), `StatusGlyph`, `Card`,
`Button`, `IconButton`, `CopyCommand`, `Countdown` (countdown + absolute, re-renders from
`clock`), `Freshness`, `Skeleton`, `Banner`, `RadioGroup`, `Switch`, `Slider`, `NumberField`,
`Dialog`, `Tooltip`, `Icon` (inline sprite), `LiveRegion`, `ViewHeader`.
`App.svelte` mounts the header, the router outlet with the three screen files (owned by
T09/T10), the single live region, theme application before first paint, global shortcuts.

**Acceptance tests.**
- `format` passes every vector in `format-vectors.json`; property tests with fast-check are
  not required, the vectors are.
- Contrast test computed from `tokens.css` for both themes meets D9.7 (text ≥ 4.5,
  primary ≥ 7, UI/glyph ≥ 3).
- Every primitive: Testing Library tests for each state and keyboard behaviour; axe clean;
  screenshot per state x theme (Playwright harness).
- Stores: with the mock client, a pushed snapshot updates `usage`; `refresh` sets and clears
  `refreshing`; `Busy` is a no-op; `clock` stops ticking when `document.hidden`; router
  back/escape; theme override sets `data-theme` before first paint (no flash test).
- Stylelint: no raw colour, font-size or spacing outside the token files. ESLint security
  rules pass. Bundle: fonts ≤ 120 KB.
- Reduced motion and forced-colors screenshots of the primitives.

**Security rules.** D8.8 (no inline styles as strings, no `{@html}`, no `innerHTML`; CSSOM
custom properties only), D8.7 (no runtime request: fonts and icons bundled), D7.4 (types
only from the generated bindings; no hand-written IPC types).

**Depends on.** T00.

**Review checklist.**
- [ ] Components import `IpcClient`, never `@tauri-apps/api`.
- [ ] Tokens match D9.7 or the deviation was approved; no magic numbers in components.
- [ ] Status is never colour-only (glyph + word + pattern present in `Meter`).
- [ ] `Meter` and `Countdown` have correct `aria-valuetext`; live region is single and polite.
- [ ] No timer runs while the document is hidden.
- [ ] Fixtures cover every `UsageView` state, every `Unavailable` and `Setup` value.

---

## T08 — Usage service: the one loop (wave 2)

**Goal.** Wire sources, store, settings, alerts and the snapshot channel into the single
event-driven task of D7.1–D7.2.

**Owns.** `dashboard/crates/usage-io/src/{registry.rs,service.rs}`,
`dashboard/crates/usage-io/tests/service_*.rs`.

**Exposes.** Exactly D7.1 (`ServiceDeps`, `ServiceHandle`, `spawn`), plus:
```rust
// registry
pub struct RegistryInputs { pub usage_log: Option<PathBuf>, pub facts: Arc<FactsCell>, pub clock: Arc<dyn Clock>,
                            pub runner: Arc<ProcRunner>, pub http: Arc<HttpClient>, pub live_check: bool }
/// The one list of sources. Adding a provider is one line here.
pub fn build(i: &RegistryInputs) -> Vec<Arc<dyn UsageSource>>;
// service
pub struct Backoff;   // pure: next_delay(failures, kind, jitter: f64) -> Duration   (D7.2)
pub trait Jitter: Send + Sync { fn next(&self) -> f64; }   // injected; tests use a fixed value
```
Behaviour to implement: D7.1 loop with one timer; D7.2 scheduling, visibility, backoff,
one-in-flight, manual refresh semantics (`live_check` target allowed only when the setting
is on and the source's cost is `SpendsQuota`, else `NotEnabled`); merge of several sources
per provider by greatest `observed_at`; `assess::derive` per provider; `store.record` and
`alerts::step` + `notifier.notify` + `save_alerts` on each new reading; snapshot emitted
only when `view::same_content` is false; re-probe `EngineFacts` at start, every 30 minutes
while visible, and on manual refresh; first-run backfill once (`MetaKey::BackfillDone`),
with alerts suppressed until it finishes; daily prune; settings changes applied live
(thresholds re-evaluate severity and headline at once; hiding a provider stops its
fetches); graceful stop (drop of all handles ends the task, in-flight fetches cancelled).

**Acceptance tests** (fake sources, `FakeClock`, tokio paused time, in-memory or temp store).
- Start: all `Pending`, then each provider settles as its fake source answers/fails.
- File-change signal → exactly one fetch of that source; 50 signals while a fetch is in
  flight → one more fetch after it.
- Interval source: fetched at the visible interval when visible, hidden interval when
  hidden; becoming visible with an old reading triggers one fetch; never two in flight.
- Backoff sequence 1, 2, 4 … 60 min with injected jitter; reset on success; `RateLimited`
  starts at 15 min; manual refresh bypasses it; second manual while busy → `Busy`.
- Clock jump forward by 8 h (suspend): each due source runs once, not eight times.
- Failure after success → `Stale{Failing}` keeps the reading; quiet passive source →
  `Stale{Quiet}` after 10 min; window passing `resets_at` flips to elapsed and the headline
  changes **with no fetch** (timer-driven).
- Alerts: crossing 80 % notifies once; staying above does not repeat; restart with the same
  store does not repeat; critical only when both crossed; none when disabled; none for
  stale data; none during backfill.
- `live_check` refused when disabled; allowed when enabled; never scheduled by a timer.
- Snapshot events: identical content → no event. **Idle proof**: with all sources quiet
  and no deadline within an hour, advancing paused time by 59 minutes runs zero task wakes
  (instrumented loop counter).
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
- [ ] Exactly one `sleep_until`; no `interval` ticking when nothing is due.
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
stale-failing / elapsed / unavailable / not-configured, `FirstRun`, `OfflineBanner`,
`UnsupportedLine`), `dashboard/src/lib/overview/**` (pure helpers: `orderProviders`,
`isAllNetworkOffline`, `cardSummary`), their tests, `dashboard/e2e/screens/overview.spec.ts`
and its baselines.

**Consumes.** `usage`, `settings`, `clock`, `router` stores; `format`, `copy`; primitives.
No IPC calls except through the stores.

**Exposes.** `orderProviders(views: ProviderView[]): ProviderView[]` (D9.3 ordering, stable),
`cardSummary(view, nowMs, hour): string` (the card's accessible name), the screen component.

**Acceptance tests.**
- A screenshot for every fixture state x light/dark x widths 360/720/1200, plus reduced
  motion and forced colors for the mixed fixture; baselines reviewed by the lead.
- axe clean on every state; each card is one labelled button with the summary as its name;
  inner controls are separate tab stops; tab order equals reading order.
- Ordering unit tests (severity, then fixed order; no reorder on equal severity).
- Pending skeleton appears only after 150 ms and stays ≥ 300 ms (fake timers).
- Elapsed window shows "—" and the reset sentence, never a number; stale-quiet keeps full
  contrast values and shows the age; stale-failing shows cause, saved time and Retry.
- Copy button copies the exact command and announces "Copied"; when the clipboard API
  rejects, the command text becomes selected and the hint changes.
- Offline banner appears only when every network-backed provider is `offline`; Claude's
  card is unchanged.
- First-run view shows once, then never again after any provider answers.
- 200 % zoom and 320 px width: no clipped text, no horizontal scroll.
- Rapid snapshot updates (20 per second from the mock) cause no layout shift (CLS measured
  in Playwright = 0) and no dropped frames beyond budget.

**Security rules.** D8.8 UI rules; only text from `copy.ts`, `format` and typed fields is
rendered; no dynamic HTML.

**Depends on.** T07.

**Review checklist.**
- [ ] Matches the D9.3 wireframe and D9.5 state table; numbers right-aligned in tabular mono.
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
geometry), `dashboard/src/lib/components/settings/**`, `dashboard/src/lib/stores/history.ts`,
their tests, `dashboard/e2e/screens/{detail,settings}.spec.ts` and baselines.

**Exposes.**
```ts
// lib/chart (pure, unit-tested)
export type Scale = { x(t: number): number; y(v: number): number };
export function makeScale(series: HistorySeries, width: number, height: number, pad: Pad): Scale;
export function stepPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // breaks at gaps
export function bandPath(points: HistoryPoint[], scale: Scale, gapSeconds: number): string;   // lo..hi
export function ticks(series: HistorySeries, range: HistoryRange, hour: HourCycle): { x: Tick[]; y: Tick[] };
export function summarize(series: HistorySeries, hour: HourCycle): string;                     // the role="img" label
export function nearest(points: HistoryPoint[], t: number): number;                            // index for the crosshair
// lib/stores/history.ts
export const history: { load(q: HistoryQuery): Promise<void>; readonly state: "idle" | "loading" | "ready" | "empty" | "error"; readonly series: HistorySeries | null };
```

**Acceptance tests.**
- Chart geometry: step path for known points equals an expected path string; gaps split the
  path; band for rollups; empty series → empty state ("No history yet. It builds up as you
  use the provider."); single point renders a dot; values above 100 % extend the axis.
- Chart a11y: `role="img"` label from `summarize`; hidden data table has one row per point
  (capped at 600); Left/Right move the crosshair and announce the value; pointer hover shows
  the same readout; axe clean.
- Range switch is a radio group with arrow keys; changing range/meter issues exactly one
  `getHistory` and cancels a stale response (out-of-order responses do not overwrite).
- Detail states: provider `unavailable`/`not_configured`/`unsupported` show the state block
  and no empty chart frame; `stale` shows the freshness note.
- Live check button exists only when the setting is on and `manual === "spends_quota"`;
  it states the cost; `NotEnabled`/`Busy` handled.
- Settings: every control reads and writes through `settings.update`; thresholds validate
  on blur (warn < critical, 50..100) with a specific message; sliders and number fields stay
  in sync; bar tick marks elsewhere move live; "Saved" tick announced; a failed save rolls
  the control back and says so; enabling the live check opens the confirmation dialog
  (focus trapped, Esc cancels, focus returns); tray toggle disabled with a reason when
  `tray_available` is false; diagnostics list renders `SourceDiag`.
- Screenshots: detail (two meters, one meter, balance, each range, empty, error) and
  settings x themes x widths; reduced motion; forced colors.
- Keyboard walk-through of both screens recorded in the PR notes.

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
`localtime.rs`, `cli.rs`), `dashboard/src-tauri/tests/**`.

**Exposes / implements.**
```rust
// commands.rs — exactly these six, names fixed by the capability file
#[tauri::command] fn get_snapshot(svc: State<'_, ServiceHandle>) -> Snapshot;
#[tauri::command] async fn refresh(svc: State<'_, ServiceHandle>, target: RefreshTarget) -> Result<(), CommandError>;
#[tauri::command] async fn get_history(svc: State<'_, ServiceHandle>, query: HistoryQuery) -> Result<HistorySeries, CommandError>;
#[tauri::command] fn get_settings(svc: State<'_, ServiceHandle>) -> Settings;
#[tauri::command] async fn update_settings(app: AppHandle, svc: State<'_, ServiceHandle>, patch: SettingsPatch) -> Result<Settings, CommandError>;
#[tauri::command] fn get_diagnostics(svc: State<'_, ServiceHandle>) -> Diagnostics;
// events.rs
pub const EVENT_SNAPSHOT: &str = "snapshot"; pub const EVENT_NAVIGATE: &str = "navigate";
pub fn forward_snapshots(app: AppHandle, rx: watch::Receiver<Snapshot>);   // emits only to the "main" window, only while it exists
// tray_icon.rs (pure)
pub struct IconSpec { pub used_step: u8 /* 0..=20, 5 % steps */, pub level: Severity }
pub fn render(spec: IconSpec, size: u32) -> Vec<u8>;                        // RGBA, D9.11 shapes
// tray.rs
pub fn install(app: &AppHandle) -> Result<TrayState, TrayError>;            // TrayError is a field-less enum
pub fn update(app: &AppHandle, snapshot: &Snapshot, local: &dyn LocalTime, hour: HourCycle);   // rows from core::text::tray_rows
pub fn host_available() -> bool;                                            // StatusNotifierWatcher present on the session bus
// notifier.rs
pub struct DesktopNotifier; impl Notifier for DesktopNotifier              // title/body from core::text::notification
// window.rs
pub fn open(app: &AppHandle); pub fn close_requested(app: &AppHandle);      // hide, destroy after 30 s, or quit when no tray
pub fn remember_size(app: &AppHandle, w: u32, h: u32);                      // debounced into settings
// autostart.rs
pub fn desktop_entry(exec: &Path) -> String;                                // pure
pub fn set(enabled: bool) -> std::io::Result<()>;                           // ~/.config/autostart/dev.jev.dashboard.desktop, atomic
// hardening.rs
pub fn apply();                                                             // umask 077, PR_SET_DUMPABLE 0, panic hook (location only)
// localtime.rs
pub struct SystemLocalTime; impl LocalTime for SystemLocalTime             // chrono Local
```
`main.rs` order: `hardening::apply()` → single-instance (second launch calls
`window::open`) → build `ServiceDeps` (engine_bridge paths, `SqliteStore`,
`JsonSettingsStore`, `registry::build`, `watch_file`, `DesktopNotifier`) → `spawn` → tray →
window unless `--hidden` → run loop with `ExitRequested` prevented while the tray exists.
Service visibility follows the window (`set_visible`). Navigation locked to the app origin;
new windows denied; context menu and reload disabled in release. The web view is created on
open and destroyed 30 s after close (D9.13).

**Acceptance tests.**
- `tray_icon::render`: golden-image tests for representative specs (0, 50, 80, 95, 100 % x
  levels); property: every pixel buffer has length `size*size*4`, alpha-premultiplied
  correctly, deterministic.
- `autostart::desktop_entry` golden text; `set(true/false)` in a temp `XDG_CONFIG_HOME`.
- `hardening::apply`: after the call, a created file is 0600 and `/proc/self/status` shows
  the process is not dumpable (test in a child process); panic hook output contains the
  location and not the payload (child process with a canary payload).
- Command layer with `tauri::test` mock runtime: each command returns the handle's value;
  invalid arguments (unknown provider, malformed meter id, out-of-range thresholds) return
  `invalid_argument`; payload JSON equals the bindings' shape.
- Snapshot tests: capability file, CSP, plugin list, absence of the `devtools` feature,
  `withGlobalTauri: false`, exactly six commands registered.
- Window lifecycle with the mock runtime: close → hidden → destroyed after 30 s (paused
  time); open recreates; `--hidden` creates no window; no tray host → close quits.
- Manual check list (recorded in the PR): tray icon and menu on this machine's host,
  notification appears once, second launch focuses the window, size is remembered, light and
  dark follow the OS, the app runs beside a live `jev-claude` session with no effect on it.

**Security rules.** D8.8 in full; D8.4 (no logging except `diag`; panic hook); D7.3 (no
extra command, no event from UI to Rust); notifications and tray text only from
`core::text`; the shell reads no file itself except through `fsutil`/stores.

**Depends on.** T08 (and the event/command names used by T07's `tauri.ts`).

**Review checklist.**
- [ ] No logic in the shell that belongs in core/io (no formatting, no thresholds, no parsing).
- [ ] Exactly six commands; capability and CSP snapshots unchanged from T00.
- [ ] Web view destroyed when closed; service marked hidden; no timer in the shell except
      the 30 s destroy and the size debounce.
- [ ] No plugin added; no `unsafe`.
- [ ] Tray works with menu-only interaction; no-tray fallback implemented.

---

## T12 — Security proofs, CI, end-to-end, budgets (wave 4)

**Skills.** Run `security-review` on the complete dashboard diff and fix or report every
finding.

**Goal.** Make every rule of D8 and D11 mechanically enforced, and finish `scripts/ci.sh`.

**Owns.** `scripts/ci.sh` (takes over from T00), `deny.toml` tuning (with lead approval for
any allow), `dashboard/crates/usage-io/tests/{architecture.rs,canary_e2e.rs,env_scrub.rs}`,
`dashboard/crates/usage-core/tests/ipc_audit.rs`, `dashboard/fuzz/**`,
`dashboard/e2e/webdriver/**`, `dashboard/e2e/screens/security.spec.ts`,
`dashboard/scripts/measure-idle.sh`, coverage configuration.

**Delivers.**
- Every proof in the table of D8.10 that is not already owned by T01–T11, in particular:
  the **architecture test** (allowed locations of `expose_secret`, `reqwest`, process APIs,
  `jev_router::` paths; forbidden file names; forbidden macros; no `String` in error types),
  the **IPC audit** (key allow-list, forbidden key pattern, string-leaf vocabulary, over
  proptest-generated values of every IPC type and over the generated `.ts` files), the
  **end-to-end canary** (service + real adapters over stubs + real store in a temp state
  dir; search snapshots, all files' raw bytes, `diag` capture), the **no-network** Playwright
  test (any request other than app origin/IPC fails; CSP violation events fail).
- Fuzz targets for the seven entry points of D11.2 with seed corpora from the fixtures, and
  the stable byte-level proptests that mirror them.
- Real-build e2e (`tauri-driver` + `WebKitWebDriver` when present): start the `dist` binary
  with `XDG_STATE_HOME`/`HOME` pointing at a temp tree containing a synthetic `usage.jsonl`
  and a `PATH` that contains only stub `codex` and `claude` programs (no real CLI is ever run); assert the overview shows the fixture numbers; append a row and
  assert the UI updates within 3 s; open detail, settings; change a threshold; verify
  `settings.json` and that the temp tree contains no canary. If the driver is missing, the
  stage prints SKIPPED with install instructions and `--release` fails.
- `measure-idle.sh`: launches the binary hidden for 10 minutes against the temp tree and
  reports CPU time, wakeups (`/proc/<pid>/status` context switches), RSS, and binary size
  against the D11.3 budgets.
- `ci.sh`: all twelve stages of D11.4, `--bootstrap`, `--release`, a final table with
  PASS/FAIL/SKIPPED per stage and total time; non-zero exit on any FAIL (and on SKIPPED with
  `--release`).

**Acceptance.** `scripts/ci.sh` green on this machine; a deliberate violation of each rule
(a `println!`, a second `expose_secret`, an extra IPC field named `email`, a string leaf, an
extra capability permission, a remote `<img>`) makes the matching stage fail (demonstrated in
the report, not committed); coverage thresholds met or the gap reported per file.

**Security rules.** All of D8. This task may not weaken a rule to make a test pass; a rule
that cannot be met is escalated to the lead.

**Depends on.** T01–T11.

**Review checklist.**
- [ ] Each row of D8.10 maps to a named, passing test (table in the report).
- [ ] Mutation check done: each proof fails when its rule is broken.
- [ ] `ci.sh` has no silent skip; engine stage first and unchanged.
- [ ] `cargo deny`/`audit`/`pnpm audit` clean, or each exception justified and time-boxed.
- [ ] Idle budgets measured and reported with numbers.

---

## T13 — Packaging and final UI polish audit (wave 4, UI-design pass)

**Skills.** `impeccable` (audit, then polish), `better-ui`.

**Goal.** Installable on Arch, and a last expert pass over the whole UI against D9.

**Owns.** `dashboard/scripts/install.sh`, `dashboard/packaging/dev.jev.dashboard.desktop`,
`dashboard/src-tauri/icons/**` (app icon set, designed here), `dashboard/README.md`
(install, autostart, optional Hyprland window rule, privacy summary, troubleshooting),
`dashboard/CHANGELOG.md`; and, for the polish pass only, edits to UI files under
`dashboard/src/**` after T09/T10 are merged (list every touched file in the report; no
behaviour or contract changes; baselines regenerated and reviewed).

**Delivers.**
- D12 build and install flow; `install.sh [dest]` and `--uninstall`; desktop entry
  validated with `desktop-file-validate`; icons in hicolor sizes; the script touches nothing
  else and the root `scripts/install.sh` is unchanged.
- An `impeccable` audit report against D9 (hierarchy, spacing rhythm, alignment of numerals,
  state coverage, copy tone, motion, contrast, focus, 320 px and 200 % zoom) with each
  finding fixed or listed for the lead; a `better-ui` pass (concentric radii, optical
  alignment, hit areas, surface depth).
- Orca read-through notes and a keyboard-only walk-through of the installed app.

**Acceptance.** Fresh install into a temp prefix runs; uninstall leaves no file; the app
launches from the desktop entry and with `--hidden`; `scripts/ci.sh --release` still green
with regenerated baselines; bundle and idle budgets still met.

**Security rules.** The install script uses no `sudo`, no network, no `curl | sh`, writes
only under the given prefix and `~/.local/share`; the README's privacy section states
exactly what D8 guarantees and its honest limits.

**Depends on.** T09, T10, T11 (runs alongside T12; T12 owns `ci.sh`, T13 does not edit it).

**Review checklist.**
- [ ] `scripts/install.sh` byte-identical to before the project.
- [ ] Desktop entry valid; icon legible at 16–128 px in light and dark.
- [ ] Polish diff changes presentation only; no new dependency; screenshots reviewed.
- [ ] README privacy claims match D8 exactly (no overclaiming).

---

## 2. Lead review protocol (for every task)

1. Run `scripts/ci.sh`. Read the diff in full; check file ownership against this document.
2. Walk the task's checklist; for security rules, open the named test and confirm it would
   fail if the rule were broken.
3. Search the diff for: new dependencies, `allow(` attributes, `String` in error or IPC
   types, `unwrap`/`expect`, new file names read from disk, new network hosts, new commands
   or permissions, timers and loops.
4. For UI tasks: review every baseline screenshot in both themes at 360 and 1200 px, tab
   through the screen, read the copy aloud.
5. Reject with specific, reproducible findings; accept only when D11.5 is fully met.
