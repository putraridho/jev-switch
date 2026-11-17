# Jev Dashboard: design

Status: proposed design for v1, pending the user's answers in section 17 (usage dashboard). Normative for every task in `TASKS.md`.
Date: 2026-11-17. Lead and reviewer: Fable 5.2. Implementation: parallel workers.

Words used precisely here: **MUST / MUST NOT** are review-blocking rules. **Verified** means
observed on this machine on the date above. **Assumed** means taken from documentation or
inference and not yet observed.

---

## 1. Summary

A resident desktop app (Tauri 2: Rust core, web UI) that shows, in one place, how much of each
AI plan or credit balance has been used and when it resets. v1 covers Claude Max (5-hour and
7-day windows, read from the local gateway's log) and Codex/ChatGPT (read from the free
`codex app-server` protocol). DeepSeek balance is built and activates when a key exists.
Gemini, Antigravity and Grok are typed placeholders with a documented path to real adapters.

The app is three layers with one dependency direction:

```
 web UI (Svelte)  ──IPC: 6 typed commands, 2 events, numbers and closed enums only──▶
 jev-dashboard (Tauri shell: window, tray, notifications)             thin, no logic
        │
        ▼
 jev-usage-io (adapters: log tail, subprocess, HTTP, SQLite, service loop)   IO shell
        │                         │
        ▼                         ▼
 jev-usage-core (domain, parsers, policy; pure, no IO)        jev-router (engine, reused)
```

## 1a. User decisions (2026-11-17, normative)

These answer the open questions and override any earlier default in this document.

- **Name:** the app is **Flashboard**. Binary name `jev-dashboard`. Bundle/application id `dev.jev.flashboard`. Window title, tray tooltip, `.desktop` entry and About screen say "Flashboard".
- **Claude live check:** built in v1, **off by default**, behind a setting; may be verified once with one tiny Haiku request.
- **Start at login:** **on by default**, with a clear toggle in settings (the user prefers features that work without remembering to enable them).
- **Account e-mail:** never displayed, stored, logged or sent over IPC; the UI shows the plan name only. There is no opt-in for it.
- **After v1:** the routed chat window comes next (design for it now; do not build it in v1).
- **Process rule:** every task is validated by the lead with its own tests and reviews, and review rounds repeat until a full round finds nothing. A task is not done while any review finding is open.

## 2. Goals, non-goals, user stories

### Goals (v1)
1. One glance answers "how close am I to a limit, and when does it reset?" for every plan.
2. Works beside `jev-claude` with zero interference: read-only on the gateway's files, no
   requests through the gateway, no model calls unless the user opts in.
3. Resident and cheap: lives in the tray, no busy loops, no periodic UI work while hidden.
4. Never surfaces a prompt, token, key, e-mail or account id anywhere (section 8).
5. A typed core that the later routed chat window can reuse unchanged.

### Non-goals (v1)
- The chat window (designed for in section 15, not built).
- Showing account identity (e-mail). v1 shows the plan name only. An opt-in identity display
  is an open question (section 17), not built.
- Cost accounting per token, per project or per session. The gateway log has that data next
  to prompt-adjacent metadata; v1 deliberately does not read it.
- Windows and macOS packaging. The code stays portable; only Linux is tested and shipped.
- Auto-update, telemetry, crash upload, remote configuration. Never.
- Editing Hyprland or any other user configuration.

### User stories
- US1. As a daily `jev-claude` user I open the dashboard and see Claude 5h and 7d usage as a
  percentage with "resets in 2h 14m (today 14:00)", updated within about 2 seconds of my last
  request, without doing anything.
- US2. I see Codex usage and reset time without opening Codex.
- US3. The tray icon shows my highest usage at a glance and changes shape and colour at 80%
  and 95% (configurable).
- US4. I get one notification when a window crosses a threshold, never repeated for the same
  window, and none when I have notifications off.
- US5. When data is old, missing or failing I am told exactly which, why, and what to do
  (copyable command), never a blank card or a fake number.
- US6. I open a provider and see its history for the last day, week, month or quarter.
- US7. I add `DEEPSEEK_API_KEY` to `~/.jev-router.env` and the DeepSeek card starts showing
  my balance; the key never appears anywhere.
- US8. The app starts hidden at login, remembers its window size, follows the OS light/dark
  theme, and is fully usable from the keyboard and a screen reader.

## 3. Verified facts (spikes run 2026-11-17)

### 3.1 Codex app-server (verified, free, no model call)
- `codex-cli 0.169.0`. `codex app-server` speaks newline-delimited JSON-RPC 2.0 on
  stdin/stdout by default. Responses omit the `jsonrpc` field. Unsolicited notifications
  (`remoteControl/status/changed`, `account/updated`) arrive between responses.
- Sequence that works: `initialize {clientInfo:{name,version}}` → notification `initialized`
  → `account/rateLimits/read {excludeResetCreditDetails:true}` → close stdin.
- Timings (one process): initialize 212 ms, `account/rateLimits/read` 613 ms,
  `account/usage/read` 831 ms, `account/read` 4 ms. Exit code 0 about 15 ms after stdin
  closes. Whole session 2.75 s. Stderr empty.
- Real response on the FREE plan (ids redacted), recorded as fixtures in
  `dashboard/fixtures/codex-app-server/` (README there lists every redaction):
  ```json
  {"ordinaryUsageAllowed":true,
   "rateLimits":{"limitId":"codex","limitName":null,"normalModelSlug":null,
     "primary":{"usedPercent":4,"windowDurationMins":43200,"resetsAt":1797439066},
     "secondary":null,
     "credits":{"hasCredits":false,"unlimited":false,"balance":null},
     "individualLimit":null,"spendControlReached":false,"planType":"free",
     "rateLimitReachedType":null},
   "rateLimitsByLimitId":{"codex":{ ...same snapshot... }},
   "rateLimitResetCredits":{"availableCount":0,"credits":[]},
   "accountId":"<redacted>","rateLimitUpsell":null}
  ```
  The free plan has ONE window of 43200 minutes (30 days) and `secondary: null`. `usedPercent`
  is an integer 0..100. `resetsAt` is epoch seconds, nullable.
- `account/usage/read` returns `{"summary":{"lifetimeTokens":0,"peakDailyTokens":0,
  "longestRunningTurnSec":null,"currentStreakDays":0,"longestStreakDays":0},
  "dailyUsageBuckets":[],"threadUsage":null}`. Not needed for v1.
- `account/read` returns e-mail, `userId` and `chatgptAccountId`. **The dashboard MUST NOT
  call it**: the plan name is already in `rateLimits.planType`.
- `account/rateLimits/updated` exists (sparse rolling update) but is only expected during
  turns run by that same server. The dashboard runs no turns, so it polls with a short-lived
  process instead of keeping a server resident.
- Protocol schema: `codex app-server generate-json-schema --out DIR`.
- `codex login status` prints `Logged in using ChatGPT` (already used by the engine's probe).

### 3.2 Claude gateway log (verified)
- `~/.local/state/jev-router/usage.jsonl`: one JSON object per line, about 2.8 KB per line
  (max seen 3.3 KB), 0600, rotated to `usage.jsonl.1` at 50 MB (`JEV_LOG_MAX_MB`). Path is
  `jev_router::log::path_from_env("JEV_USAGE_LOG", "usage.jsonl")`.
- In the last 4 MB: 1425 rows, 1336 with a non-empty `ratelimit` object. Latest sample was
  14 seconds old (a `jev-claude` session was active). Samples are produced ONLY by requests
  that pass through the gateway; plain `claude`, claude.ai and the mobile app consume the same
  plan without producing samples.
- `ts_ms` is an integer (epoch ms). Every `ratelimit` value is a **string**:

  | key (prefix `anthropic-ratelimit-unified-`) | example | meaning |
  |---|---|---|
  | `5h-utilization`, `7d-utilization` | `"0.5"`, `"0.37"` | fraction used, 0..1 |
  | `5h-reset`, `7d-reset` | `"1794898800"` | epoch seconds |
  | `5h-status`, `7d-status` | `"allowed"` | window status |
  | `status`, `reset` | `"allowed"`, `"1794898800"` | overall |
  | `representative-claim` | `"five_hour"` | which window binds |
  | `fallback-percentage` | `"0.5"` | not used in v1 |
  | `overage-status`, `overage-disabled-reason` | `"rejected"`, `"org_level_disabled"` | not used in v1 |
  | `7d_oi-utilization/-reset/-status` | seen on 8 of 1336 rows | an extra window; meaning unknown |

- Other top-level keys in a row include `cwd`, `project`, `session`, `prompt_id`,
  `message_id`, `request_id`, `model`, `params`, `usage`, `error`, `tools_called`. These are
  prompt-adjacent. `gateway.jsonl` and `turns.jsonl` contain prompt text. See section 8.3.
- `claude -p --output-format stream-json` emits `rate_limit_event` with
  `rate_limit_info.unifiedWindows.{five_hour,seven_day}.{utilization,resetsAt,status}`
  (fixture `tests/fixtures/claude/turn1.jsonl`). Fixture only: no live call was made.

### 3.3 Gemini, Antigravity, Grok (docs and `--help` only)
| provider | conclusion | detail |
|---|---|---|
| Gemini CLI | **not available** headless | `gemini --help` (run) has no usage or quota command or flag. Quota is shown only by the interactive `/stats`. An undocumented internal Code Assist quota call exists; using it would require reading the OAuth token, which this app never does. |
| Antigravity (`agy`) | **available per docs, unverified here** | Docs: `/usage` shows per-model-group quota; reported to work headless as `agy -p /usage --output-format json` with `remaining_fraction` and `reset_time` per bucket, about 7 s, no quota spent. `agy` on this machine is a **lazy mise shim that installs on first run** (see 3.5), so it was not executed to completion. |
| Grok | **partly available, not for v1** | xAI Management API (documented): `GET https://management-api.x.ai/v1/billing/teams/{team_id}/prepaid/balance` with a separate management key gives API credit balance. The Grok Build subscription quota is only shown by the CLI's `/usage` through a non-public endpoint that needs the token in `~/.grok/auth.json`, which this app never reads. `grok` is never executed. |

Sources: antigravity.google/docs/cli/commands/usage, github.com/google-antigravity/antigravity-cli
issue 46, docs.x.ai/developers/rest-api-reference/management/billing,
github.com/steipete/CodexBar docs/xai.md.

### 3.4 Toolchain (verified)
- `pnpm dlx @tauri-apps/cli@latest info` works: Arch rolling, Hyprland on Wayland,
  webkit2gtk-4.1 2.54.1, rsvg2 2.63.3, rustc/cargo 1.97.0 (stable only, no nightly),
  node 22.20.0, pnpm 10.34.4, bun 1.4.3. "Packages / Plugins / App" are empty (no app yet).
- `libayatana-appindicator 0.6.1` is installed. A StatusNotifierWatcher/Host and
  `org.freedesktop.Notifications` are on the session bus (Omarchy's quickshell), so a tray
  icon and notifications have a host.
- NOT installed: `cargo-deny`, `cargo-audit`, `cargo-llvm-cov`, `cargo-fuzz`, `tauri-driver`,
  `WebKitWebDriver` (not in the Arch `webkit2gtk-4.1` package). Section 11 handles each.

### 3.5 Incident during the spike (disclosed)
Running `agy --help` under a 15 s timeout started a mise download of `antigravity-cli@1.5.9`
(the brief warned only about `grok`; `agy` is the same kind of lazy shim). The timeout killed
it at about 7 of 99 MB. It left an empty directory
`~/.local/share/mise/installs/antigravity-cli/1.5.9`, which was not removed. Rule added to
the design: **never execute `agy` or `grok`; gate every subprocess on the engine's
availability probe, which decides from the filesystem** (section 8.6).

## 4. Domain model (normative)

Lives in `jev-usage-core`. Pure data, no IO, `#![forbid(unsafe_code)]`. Every type below
derives only what is listed. "IPC" marks types that derive `serde::Serialize` and
`ts_rs::TS` and may cross to the web UI. Types without that mark cannot cross, by
construction.

### 4.1 Primitives
```rust
// time.rs
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct UnixMillis(pub u64);
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct UnixSeconds(pub u64);
impl UnixMillis { pub fn seconds(self) -> UnixSeconds; pub fn saturating_since(self, earlier: Self) -> Duration; }
/// The only source of "now". Nothing in core or io calls SystemTime directly.
pub trait Clock: Send + Sync { fn now(&self) -> UnixMillis; }

// quantity.rs
/// Usage in basis points (1/100 of a percent). 0..=60_000 (600 %); larger inputs saturate.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Percent(u16);
impl Percent {
    pub const ZERO: Self; pub const FULL: Self;          // 0 and 10_000
    pub fn from_basis_points(bp: u32) -> Self;           // saturating
    pub fn from_fraction(f: f64) -> Option<Self>;        // None for NaN, inf, negative
    pub fn from_whole(percent: i64) -> Option<Self>;     // None for negative
    pub fn basis_points(self) -> u16;
}
/// Money in millionths of the currency unit, parsed exactly from a decimal string.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Money(i64);
impl Money { pub fn parse_decimal(s: &str) -> Option<Self>; pub fn micros(self) -> i64; }
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)] pub enum Currency { Usd, Cny, Other }   // IPC

// label.rs
/// The ONLY free-form string in the domain. 1..=40 bytes of [a-z0-9_]. Used for plan names
/// ("max_5x", "free") and unknown window names ("7d_oi"). Too short and too restricted to
/// carry a prompt, a key, an e-mail or a path.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Label(Box<str>);
impl Label { pub fn parse(s: &str) -> Option<Self>; pub fn as_str(&self) -> &str; }

// provider.rs
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]                       // IPC
pub enum ProviderKey { Claude, Codex, DeepSeek, Gemini, Antigravity, Grok }
impl ProviderKey { pub const ALL: [Self; 6]; pub const fn as_str(self) -> &'static str; }
```
`ProviderKey` is the dashboard's own list (Antigravity is separate from Gemini for billing).
The mapping to the engine's `ProviderId` lives in `jev-usage-io::engine_bridge`, so core does
not depend on the engine.

### 4.2 Meters and readings
```rust
// meter.rs
#[derive(Clone, Eq, PartialEq, Hash, Debug)]                                              // IPC
pub enum WindowKind { FiveHour, SevenDay, Monthly, Rolling { minutes: u32 }, Named(Label) }
impl WindowKind { pub fn from_minutes(m: u64) -> Self; }   // 300, 10080, 43200 map to the named kinds

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum MeterId { Window(WindowKind), Balance(Currency) }
impl MeterId {
    /// Stable storage/IPC key: "w:5h" "w:7d" "w:30d" "w:r:<minutes>" "w:n:<label>" "b:usd" "b:cny" "b:other".
    pub fn key(&self) -> String;
    pub fn parse_key(s: &str) -> Option<Self>;             // exact inverse of key()
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]                                        // IPC
pub enum LimitStatus { Allowed, Warning, Rejected, Unknown }

#[derive(Clone, PartialEq, Debug)]
pub struct WindowReading { pub kind: WindowKind, pub used: Percent,
                           pub resets_at: Option<UnixSeconds>, pub status: LimitStatus }
#[derive(Clone, PartialEq, Debug)]
pub struct BalanceReading { pub currency: Currency, pub total: Money,
                            pub granted: Option<Money>, pub topped_up: Option<Money>, pub usable: bool }
#[derive(Clone, PartialEq, Debug)]
pub enum Meter { Window(WindowReading), Balance(BalanceReading) }
impl Meter { pub fn id(&self) -> MeterId; }

/// 1..=8 meters with unique ids. The field is private: an empty or duplicate set cannot exist.
#[derive(Clone, PartialEq, Debug)]
pub struct Meters(Vec<Meter>);
impl Meters { pub fn new(v: Vec<Meter>) -> Option<Self>; pub fn as_slice(&self) -> &[Meter]; }

#[derive(Clone, PartialEq, Debug)]
pub struct Reading { pub meters: Meters, pub plan: Option<Label>, pub observed_at: UnixMillis }
```

### 4.3 Usage: the state of one provider
```rust
// usage.rs
#[derive(Clone, PartialEq, Debug)]
pub enum Usage {
    /// No source has answered yet in this run.
    Pending,
    /// A reading that is current by the provider's freshness policy.
    Known(Reading),
    /// The last good reading, kept, with the instant it stopped being current and why.
    Stale { last: Reading, since: UnixMillis, cause: StaleCause },
    /// No reading to show.
    Unavailable { reason: Unavailable },
    /// The user has to do something first.
    NotConfigured { setup: Setup },
    /// No usage interface exists for this provider (placeholders).
    Unsupported,
}
#[derive(Copy, Clone, Eq, PartialEq, Debug)]                                              // IPC
pub enum StaleCause { Quiet, Failing(Unavailable) }  // Quiet: passive source saw nothing new
#[derive(Copy, Clone, Eq, PartialEq, Debug)]                                              // IPC
pub enum Unavailable { NotInstalled, NotLoggedIn, Offline, Timeout, RateLimited,
                       Unauthorized, ServerError, Protocol, Io, Internal }
#[derive(Copy, Clone, Eq, PartialEq, Debug)]                                              // IPC
pub enum Setup { UseGateway, InstallCli, LogIn, AddDeepSeekKey }
```
There is no `Known` without a reading, no `Stale` without the reading it refers to, no error
that carries a string. A provider cannot be "known and failing" at once; it is `Stale` with
`Failing(reason)`.

`Usage` is **derived**, never stored: `assess::derive(&SourceState, now, &FreshnessPolicy)`
is a pure function over what the service remembers
(`SourceState { last_good: Option<Reading>, last_failure: Option<(UnixMillis, SourceError)>, attempted: bool }`).

### 4.4 Ports (traits core defines, io implements)
```rust
// port.rs
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum SourceId { ClaudeGatewayLog, ClaudeLive, CodexAppServer, DeepSeekBalance,
                    GeminiNone, AntigravityNone, GrokNone }
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Trigger {
    /// Fetch when the watched file changes (plus once at start).
    FileChanged,
    /// Fetch on a timer; the interval depends on whether the window is visible.
    Interval { visible: Duration, hidden: Duration },
    /// Fetch only when the user asks.
    Manual,
}
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum Cost { Free, SpendsQuota }          // IPC
#[derive(Copy, Clone, Debug)]
pub struct SourceDescriptor { pub id: SourceId, pub provider: ProviderKey,
                              pub trigger: Trigger, pub cost: Cost, pub timeout: Duration }
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum SourceError { NotConfigured(Setup), Unavailable(Unavailable), Unsupported }

pub type FetchFuture<'a> = Pin<Box<dyn Future<Output = Result<Reading, SourceError>> + Send + 'a>>;
/// One way to learn a provider's usage. Adding a provider is adding one implementation and
/// one line in the registry; nothing else changes (open/closed).
pub trait UsageSource: Send + Sync {
    fn descriptor(&self) -> SourceDescriptor;
    fn fetch(&self) -> FetchFuture<'_>;
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum StoreError { Unavailable, Corrupt }
pub trait HistoryStore: Send + Sync {
    fn record(&self, provider: ProviderKey, reading: &Reading) -> Result<(), StoreError>;
    fn series(&self, provider: ProviderKey, meter: &MeterId, from: UnixSeconds, to: UnixSeconds,
              max_points: u16) -> Result<Vec<SamplePoint>, StoreError>;
    fn load_alerts(&self) -> Result<AlertMemory, StoreError>;
    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError>;
    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError>;
}
pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Settings;                              // defaults when missing or invalid
    fn save(&self, s: &Settings) -> Result<(), StoreError>;
}
pub trait Notifier: Send + Sync { fn notify(&self, alert: &Alert); }
```
A provider may have several sources (Claude: gateway log and the opt-in live check). The
service keeps the reading with the greatest `observed_at`.

### 4.5 Policy (pure functions in core)
- `assess::derive(state, now, policy) -> Usage` and `assess::phase(window, now) -> Phase`
  (`Current` or `Elapsed` when `resets_at <= now`).
- `assess::severity(used, thresholds) -> Severity` (`Ok | Warn | Critical`), and `Unknown` for
  an elapsed or absent meter.
- `alerts::step(memory, provider, reading, now, settings) -> (AlertMemory, Vec<Alert>)`:
  section 7.6.
- `summary::headline(&[ (ProviderKey, Usage) ], now, thresholds) -> Option<Headline>`: the
  window meter with the highest `used` among `Known` and non-elapsed `Stale` readings of
  enabled providers.
- `history::should_store(last_stored, candidate) -> bool`, `history::downsample(points, max)`.
- `text::*`: the only place tray and notification strings are built (section 9.9).
- `settings::Settings::validated(raw) -> Settings`.

### 4.6 SOLID mapping
| principle | how it is met |
|---|---|
| Single responsibility | One module, one reason to change: each `parse::*` knows one wire format; `assess` knows freshness; `alerts` knows thresholds; `tail` knows files; `store` knows SQL; `tray` knows the tray. |
| Open/closed | New provider = new `UsageSource` + one registry line + one `ProviderKey` variant. The service, store, IPC and UI render from `Meter` generically. |
| Liskov | Every `UsageSource` returns the same `Reading`/`SourceError`; the service never matches on `SourceId` to behave differently. Test: the service test-suite runs against a fake source and every real adapter driven by fixtures. |
| Interface segregation | Four small ports (`UsageSource`, `HistoryStore`, `SettingsStore`, `Notifier`) plus `Clock`. The UI gets one read model (`Snapshot`). |
| Dependency inversion | `core` defines ports and depends on nothing but serde. `io` implements them. The Tauri shell wires them. Nothing in core or io knows Tauri exists. |

DRY rules: one parser per wire format; one formatter per language with shared test vectors
(`dashboard/fixtures/format-vectors.json`, consumed by Rust `text` tests and TS `format`
tests); TypeScript IPC types are generated from Rust, never hand-written; engine facts
(state dir, env file path, env parsing, availability probes) come from `jev_router::embed`.

## 5. Repository and crate structure

**Decision: one Cargo workspace rooted at the repository, with `default-members = ["."]`.**

```toml
# Cargo.toml (root): the ONLY additions
[workspace]
members = [".", "dashboard/crates/usage-core", "dashboard/crates/usage-io", "dashboard/src-tauri"]
default-members = ["."]
resolver = "2"

[workspace.lints.rust]   # opted into by dashboard crates with `lints.workspace = true`
[workspace.lints.clippy]

[profile.dist]           # used only by the dashboard release build
inherits = "release"
lto = "thin"
codegen-units = 1
strip = true
panic = "abort"
```

Why a workspace member and not a sibling repository or a separate nested workspace:
- **DRY, one lockfile**: engine and app resolve the same `tokio`, `serde`, `reqwest`; one
  `cargo deny`/`cargo audit` pass covers both; one `target/` avoids compiling shared
  dependencies twice.
- **Root commands unchanged**: with a root package and `default-members = ["."]`,
  `cargo build --all-targets`, `cargo test` and `scripts/install.sh`
  (`cargo build --release --bin ...`) at the root select only `jev-router`, exactly as today.
  Building the app is always explicit (`-p jev-dashboard`). With resolver 2, features are
  unified only across the packages being built, so the engine's feature set does not change.
- **No change to the engine's release profile**: the app uses its own `dist` profile.
- Cost accepted: `Cargo.lock` grows by the Tauri dependency tree. `cargo build --workspace`
  now needs webkit2gtk; documented.

Layout (ownership per task is in `TASKS.md`):
```
Cargo.toml                         root manifest (+ workspace table)
src/embed.rs                       engine facade for embedding apps (new, re-exports only)
deny.toml                          cargo-deny policy (whole workspace)
scripts/install.sh                 unchanged
scripts/ci.sh                      one command that runs every gate (section 11)
docs/dashboard/                    this design, the task list
dashboard/
  crates/usage-core/               jev-usage-core   pure domain, parsers, policy, IPC DTOs
  crates/usage-io/                 jev-usage-io     adapters, store, service loop
  src-tauri/                       jev-dashboard    Tauri shell (binary)
  src/                             web UI (Svelte + TypeScript)
  fixtures/                        shared fixtures (codex-app-server/, claude-gateway/, deepseek/, format-vectors.json, ipc-allowlist.txt)
  fuzz/                            cargo-fuzz targets (optional stage; excluded from the workspace)
  e2e/                             Playwright (mock IPC) and WebDriver (real build) tests
  scripts/                         install.sh, measure-idle.sh
  package.json, pnpm-lock.yaml, .npmrc, vite.config.ts, tsconfig.json
```

### 5.1 Engine facade: `jev_router::embed`
A new module in the engine crate containing **re-exports only** (no logic), so the app's
dependency on the engine is one named, reviewable surface, and the later chat window gets the
rest through the same door:
```rust
//! The engine surface an embedding application may use.
pub use crate::backend::ProviderId;
pub use crate::providers::{Runtime, ALL_PROVIDERS};
pub use crate::availability::{probe_all, Availability, State as AvailabilityState};
pub use crate::log::{state_dir, path_from_env};
pub use crate::envfile::{env_file_path, parse as parse_env};
pub use crate::doctor::{install_hint, login_command};
```
The dashboard crates MUST import engine items only through `jev_router::embed` (enforced by
an architecture test). The engine's other code is not modified.

Known cost: `jev-router` as a library pulls `axum` and `rustyline` into the app's build
(unused code is dropped at link time). Splitting a lean `jev-engine` crate out of
`jev-router` is the right long-term fix and is deliberately deferred (section 17).

### 5.2 Module ownership
```
jev-usage-core            jev-usage-io                     jev-dashboard (shell)
  time quantity label       engine_bridge  (embed → core)    main      (wiring only)
  provider meter usage      clock          (SystemClock)     commands  (6 commands)
  port                      fsutil         (0600, atomic)    events    (emit snapshot)
  secret                    diag           (typed log)       tray      (menu + icon swap)
  parse::claude_log         tail, watch    (file tail)       tray_icon (pure rasteriser)
  parse::claude_event       proc           (subprocess)      notifier  (Notifier impl)
  parse::codex              http           (the one client)  window    (create/hide/destroy)
  parse::deepseek           secrets        (key loading)     autostart (XDG entry text)
  assess alerts summary     sources::{claude_log, claude_live, codex, deepseek, placeholder}
  history text settings     store          (SQLite)          hardening (umask, dumpable, panic hook)
  ipc                       settings_file
                            registry       (source list)
                            service        (the one loop)
```

## 6. Sources (adapters)

| source | trigger | cost | reads | never touches |
|---|---|---|---|---|
| `ClaudeGatewayLogSource` | file change on `usage.jsonl` | free | `ts_ms` and the allow-listed `ratelimit` keys | every other key; `gateway.jsonl`, `turns.jsonl`, `history`, `sessions/` |
| `ClaudeLiveSource` | manual, opt-in, off by default | **spends a tiny request** | `rate_limit_event` lines of one `claude -p` run | the answer text (discarded unparsed), `claude auth status` output |
| `CodexAppServerSource` | interval 5 min visible / 15 min hidden | free | `account/rateLimits/read` result | `account/read`, `~/.codex/*` |
| `DeepSeekBalanceSource` | interval 10 min visible / 30 min hidden | free | `GET /user/balance` body | anything else on the network |
| `PlaceholderSource` (Gemini, Antigravity, Grok) | never | free | nothing | everything (returns `Unsupported`) |

### 6.1 Claude gateway log
- Path from `embed::path_from_env("JEV_USAGE_LOG", "usage.jsonl")`; previous generation at
  `<path>.1`. Opened read-only. The source never creates, truncates, locks or writes it.
- **Cold start**: read backwards from EOF in 256 KiB blocks, at most 16 MiB, until a row with
  a parsable sample is found; if none, repeat on `<path>.1`; if none, `NotConfigured(UseGateway)`.
- **Tail**: remember `(device, inode, offset)`. On a change signal: `stat`; if the inode
  changed or the size shrank, finish the old file from the remembered offset through
  `<path>.1` when its inode matches, then start the new file at 0. Read forward in 256 KiB
  blocks, split on `\n`, keep the incomplete last line for next time. A line longer than
  1 MiB is skipped to its newline without being buffered. Memory is bounded.
- **Parsing** (`parse::claude_log::parse_line(&[u8]) -> Option<ClaudeLogSample>`): a serde
  struct with exactly two fields, `ts_ms: u64` and `ratelimit`, where `ratelimit` has a
  hand-written visitor that (a) ignores every key that does not start with
  `anthropic-ratelimit-unified-`, (b) converts each accepted value to a number or a closed
  enum immediately, (c) keeps no string except a window name that passes `Label::parse`.
  All other keys are skipped by serde without being materialised into owned values.
- A row yields one `Reading` with a window meter per window that has a utilization
  (`5h` → `FiveHour`, `7d` → `SevenDay`, anything else → `Named(label)`); the plan comes from
  the engine's availability probe (`max_5x`), not from the log.
- **First-run backfill** (once, recorded in the store's `meta`): stream `<path>.1` then
  `<path>` on a blocking thread and insert history samples under the normal `should_store`
  rule. Never repeated. Bounded by the 2 x 50 MB rotation.
- Freshness: `Known` while the newest sample is younger than 10 minutes; then
  `Stale{cause: Quiet}`; an individual window whose `resets_at` has passed is shown as
  `Elapsed` (section 9.5). This is honest: usage outside the gateway is invisible to it.

### 6.2 Claude live check (opt-in)
Runs `claude -p` once with the cheapest model and a one-word prompt and reads only
`rate_limit_event` lines (`parse::claude_event`). Disabled by default; enabling it shows a
confirmation that it spends a small request each time. Only ever triggered by the user's
click. Exact flags are confirmed by the worker from `claude --help` only, and the first real
run needs the user's explicit go-ahead (open question Q2).

### 6.3 Codex app-server
Spawn `codex app-server` (section 8.6 rules), write the three request lines of
`dashboard/fixtures/codex-app-server/requests.jsonl` (without `account/usage/read` and
without `account/read`), read lines until the response with the `rateLimits/read` id, close
stdin, wait for exit (kill after the 15 s deadline). Line cap 1 MiB, total cap 4 MiB.
`parse::codex::parse_rate_limits(&[u8]) -> Result<Reading, Unavailable>`: one window meter per
non-null `primary`/`secondary` (`WindowKind::from_minutes(windowDurationMins)`, absent
duration → `Named("primary")`/`Named("secondary")`), `plan` from `planType`,
`status = Rejected` when `rateLimitReachedType` is non-null or `ordinaryUsageAllowed` is
false. `credits.balance` (a decimal string) becomes a `Balance(Usd)` meter only when
`hasCredits` is true. Gate: only when the engine's probe says Codex is `Ready`; otherwise
`NotInstalled`/`NotLoggedIn` without spawning anything.

### 6.4 DeepSeek balance
`GET https://api.deepseek.com/user/balance`, `Authorization: Bearer <key>`. Documented body:
`{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00",
"granted_balance":"10.00","topped_up_balance":"100.00"}]}`. One `Balance` meter per currency,
amounts parsed exactly by `Money::parse_decimal`. No key → `NotConfigured(AddDeepSeekKey)`
with no network call. 401/403 → `Unauthorized`. Unverified until the user has a key.

### 6.5 Placeholders and the path to real adapters
`PlaceholderSource::new(provider)` returns `Unsupported`. The UI lists these under "No usage
interface yet" (section 9.3). Later, without touching the service or the UI:
- Antigravity: an `AntigravityUsageSource` running `agy -p /usage --output-format json`
  through `proc`, once verified against a really installed binary.
- Grok: an `XaiPrepaidBalanceSource` on the Management API with `XAI_MANAGEMENT_KEY` and
  `XAI_TEAM_ID` in the env file, through `http` (a second allowed host).
- Gemini: none until Google ships a headless quota command.

## 7. Runtime architecture

### 7.1 The service loop
`jev-usage-io::service::UsageService` is one task that owns all mutable state. No locks, no
shared mutable state, no other task mutates anything.
```rust
pub struct ServiceDeps {
    pub clock: Arc<dyn Clock>, pub sources: Vec<Arc<dyn UsageSource>>,
    pub store: Arc<dyn HistoryStore>, pub settings: Arc<dyn SettingsStore>,
    pub notifier: Arc<dyn Notifier>, pub file_changes: mpsc::Receiver<SourceId>,
}
pub struct ServiceHandle { /* cloneable */ }
impl ServiceHandle {
    pub fn snapshot(&self) -> Snapshot;                               // latest, lock-free read (watch channel)
    pub fn subscribe(&self) -> watch::Receiver<Snapshot>;
    pub async fn refresh(&self, target: RefreshTarget) -> Result<(), CommandError>;
    pub async fn history(&self, q: HistoryQuery) -> Result<HistorySeries, CommandError>;
    pub fn settings(&self) -> Settings;
    pub async fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, CommandError>;
    pub fn set_visible(&self, visible: bool);
    pub fn diagnostics(&self) -> Diagnostics;
}
pub fn spawn(deps: ServiceDeps) -> ServiceHandle;
```
The loop is a single `select!` over: the command channel, the file-change channel, completed
fetches, and **one** `sleep_until(next_deadline)` where `next_deadline` is the minimum of
every interval source's next due time, the next window reset (to flip a meter to `Elapsed`
and recompute the headline), the next freshness boundary, and the daily prune. With nothing
due, the task is parked: zero wakeups.

### 7.2 Refresh strategy
- **Event-driven first**: `watch` uses inotify (`notify` crate) on the log's parent directory
  (so rotation by rename is seen), filtered to the log's file name, and coalesces a burst
  into at most one signal per 1.5 s (trailing edge). No polling of the log, ever.
- **Intervals** only for sources with no push channel (Codex, DeepSeek), longer while the
  window is hidden. A refresh also runs when the window becomes visible if the reading is
  older than the visible interval.
- **Backoff**: on failure the next attempt is `min(60 min, 1 min x 2^failures)` with ±10 %
  jitter from an injected RNG; success resets it. `RateLimited` and `Unauthorized` start at
  15 min. A manual refresh bypasses backoff but is debounced: at most one in flight per
  source, and `Busy` is returned for a second request.
- **One fetch per source at a time**, each under its descriptor's timeout.
- **Suspend/resume and clock jumps**: every wake recomputes deadlines from the injected wall
  clock; a wake that finds deadlines in the past runs them once (no catch-up storm).
- The UI's countdown text is recomputed in the web view only while the window is visible
  (one timer aligned to the minute, stopped on `visibilitychange`). The backend never pushes
  ticks. Tray text uses absolute times, so it needs no ticking either.

### 7.3 Tauri command surface (complete)
All in `jev-dashboard::commands`. Arguments and results are the generated types of section
7.4. Errors are the closed enum `CommandError`; no command returns a string.

| command | args | result | notes |
|---|---|---|---|
| `get_snapshot` | none | `Snapshot` | synchronous read of the latest state |
| `refresh` | `target: RefreshTarget` | `Result<(), CommandError>` | result arrives as a `snapshot` event |
| `get_history` | `query: HistoryQuery` | `Result<HistorySeries, CommandError>` | at most 600 points |
| `get_settings` | none | `Settings` | |
| `update_settings` | `patch: SettingsPatch` | `Result<Settings, CommandError>` | validated in core |
| `get_diagnostics` | none | `Diagnostics` | versions, per-source state, store size |

Events (Rust → UI only): `snapshot` (payload `Snapshot`, emitted when it changes) and
`navigate` (payload `Route`, from the tray menu). The UI emits no events. There is no
command to quit, open a URL, read a file, run a process, or read the clipboard.

### 7.4 IPC types: exactly what crosses the boundary
Defined in `jev-usage-core::ipc`, `#[serde(rename_all = "snake_case")]`, tagged enums with
`#[serde(tag = "state")]`/`"kind"`. TypeScript is generated by `ts-rs` into
`dashboard/src/lib/ipc/bindings/` and checked for drift in CI. `u64`/`i64` fields carry
`#[ts(type = "number")]` (all values are far below 2^53).

```ts
type ProviderKey = "claude" | "codex" | "deep_seek" | "gemini" | "antigravity" | "grok";
type Severity    = "ok" | "warn" | "critical" | "unknown";
type Phase       = "current" | "elapsed";
type LimitStatus = "allowed" | "warning" | "rejected" | "unknown";
type Unavailable = "not_installed" | "not_logged_in" | "offline" | "timeout" | "rate_limited"
                 | "unauthorized" | "server_error" | "protocol" | "io" | "internal";
type Setup       = "use_gateway" | "install_cli" | "log_in" | "add_deep_seek_key";
type StaleCause  = { kind: "quiet" } | { kind: "failing"; reason: Unavailable };
type WindowKind  = { kind: "five_hour" } | { kind: "seven_day" } | { kind: "monthly" }
                 | { kind: "rolling"; minutes: number } | { kind: "named"; label: string };
type Currency    = "usd" | "cny" | "other";

type MeterView =
  | { kind: "window"; id: string; window: WindowKind; used_bp: number;
      resets_at: number | null;            // epoch seconds
      status: LimitStatus; severity: Severity; phase: Phase }
  | { kind: "balance"; id: string; currency: Currency; total_micros: number;
      granted_micros: number | null; topped_up_micros: number | null;
      usable: boolean; severity: Severity };
type ReadingView = { observed_at: number /* epoch ms */; plan: string | null; meters: MeterView[] };
type UsageView =
  | { state: "pending" }
  | { state: "known"; reading: ReadingView }
  | { state: "stale"; reading: ReadingView; since: number; cause: StaleCause }
  | { state: "unavailable"; reason: Unavailable }
  | { state: "not_configured"; setup: Setup }
  | { state: "unsupported" };
type ManualRefresh = "free" | "spends_quota" | "none";
type RefreshView  = { in_flight: boolean; last_attempt_at: number | null;
                      next_at: number | null; manual: ManualRefresh };
type ProviderView = { provider: ProviderKey; enabled: boolean; usage: UsageView; refresh: RefreshView };
type Headline     = { provider: ProviderKey; meter_id: string; used_bp: number;
                      severity: Severity; resets_at: number | null };
type Snapshot     = { generated_at: number; providers: ProviderView[]; headline: Headline | null };

type RefreshTarget = { kind: "all" } | { kind: "provider"; provider: ProviderKey }
                   | { kind: "live_check"; provider: ProviderKey };   // the only one that may spend
type HistoryRange  = "day" | "week" | "month" | "quarter";
type HistoryQuery  = { provider: ProviderKey; meter_id: string; range: HistoryRange };
type HistoryPoint  = { t: number /* epoch s */; v: number; lo: number; hi: number };
type HistorySeries = { provider: ProviderKey; meter_id: string; from: number; to: number;
                       unit: "basis_points" | "micros"; points: HistoryPoint[]; resets: number[] };

type Theme     = "system" | "light" | "dark";
type HourCycle = "auto" | "h12" | "h24";
type Settings  = { version: 1; warn_bp: number; critical_bp: number;
                   notify_warn: boolean; notify_critical: boolean;
                   theme: Theme; hour_cycle: HourCycle;
                   close_to_tray: boolean; autostart: boolean; claude_live_check: boolean;
                   hidden_providers: ProviderKey[] };
type SettingsPatch = Partial<Omit<Settings, "version">>;
type CommandError  = { code: "invalid_argument" | "busy" | "not_enabled" | "store_unavailable" | "internal" };
type SourceDiag    = { source: string /* closed SourceId name */; provider: ProviderKey;
                       last_ok_at: number | null; last_error: Unavailable | null; failures: number };
type Diagnostics   = { app_version: string /* semver */; sources: SourceDiag[];
                       history_bytes: number; history_samples: number; tray_available: boolean };
type Route         = { name: "overview" } | { name: "provider"; provider: ProviderKey } | { name: "settings" };
```
**String-typed leaves that cross the boundary, exhaustively**: `plan` and `WindowKind.label`
(both `Label`: `[a-z0-9_]{1,40}`), `id`/`meter_id` (`MeterId::key()`: a fixed prefix plus
digits or a `Label`), `source` (a `SourceId` variant name), `app_version` (compile-time
constant). Every other field is a number, a boolean or a closed enum. There is no field that
could hold a prompt, key, token, e-mail, account id, path, hostname, URL or error message.
`dashboard/fixtures/ipc-allowlist.txt` lists every key name; a test fails on any other
(section 8.9).

### 7.5 History store
**Decision: SQLite (`rusqlite`, bundled, no extra features) over an append-only file.**

| | SQLite | append-only file |
|---|---|---|
| crash safety, atomic writes | built in (WAL) | hand-written, the usual source of bugs |
| range queries, downsampling | indexed | full scan or a hand-written index |
| retention | `DELETE` + incremental vacuum | rewrite-and-rename compaction |
| schema change | `user_version` + migrations in a transaction | custom versioned framing |
| cost | about 1 MB binary, one C dependency | none |

The data is tiny but long-lived and must survive crashes and upgrades. The cost is paid once.

- File: `<state_dir>/dashboard/history.db` (directory 0700, files 0600 via process umask
  077, asserted by a test). `journal_mode=WAL`, `synchronous=NORMAL`,
  `auto_vacuum=INCREMENTAL`, `foreign_keys=ON`, `secure_delete=ON`.
- Schema v1 (numbers and closed-vocabulary keys only):
  ```sql
  CREATE TABLE meter (id INTEGER PRIMARY KEY, provider TEXT NOT NULL, meter_key TEXT NOT NULL,
                      UNIQUE (provider, meter_key));
  CREATE TABLE sample (meter_id INTEGER NOT NULL REFERENCES meter(id), at_s INTEGER NOT NULL,
                       value INTEGER NOT NULL, resets_at_s INTEGER,
                       PRIMARY KEY (meter_id, at_s)) WITHOUT ROWID;
  CREATE TABLE rollup_hour (meter_id INTEGER NOT NULL REFERENCES meter(id), hour_s INTEGER NOT NULL,
                            lo INTEGER NOT NULL, hi INTEGER NOT NULL, last INTEGER NOT NULL,
                            PRIMARY KEY (meter_id, hour_s)) WITHOUT ROWID;
  CREATE TABLE alert_memory (meter_id INTEGER NOT NULL REFERENCES meter(id), level INTEGER NOT NULL, -- 1 warn, 2 critical
                             fired_at_s INTEGER NOT NULL, window_resets_at_s INTEGER,
                             PRIMARY KEY (meter_id, level)) WITHOUT ROWID;
  CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID;
  ```
  `provider` is `ProviderKey::as_str()`, `meter_key` is `MeterId::key()`. `value` is basis
  points for windows and micros for balances.
- Write rule (`history::should_store`): store when the value or `resets_at` differs from the
  last stored sample of that meter, or when 15 minutes have passed since it (a heartbeat, so
  the chart can tell "flat" from "no data").
- Retention: raw samples 45 days (covers the 30-day Codex window); hourly rollups
  (`lo/hi/last`) 400 days; prune at start and every 24 h in one transaction, then
  `PRAGMA incremental_vacuum`. Expected size: well under 5 MB.
- Migration: `PRAGMA user_version`; an ordered list of `(version, sql)` applied in one
  transaction; a database newer than the binary is opened read-only for charts and not
  written; a corrupt database is renamed to `history.db.corrupt-<epoch>` and recreated
  (history is derived data; losing it never breaks the app).
- Access: one connection behind a mutex, every call through `spawn_blocking`.

### 7.6 Alerts (pure state machine)
For each window meter and each enabled threshold (warn, critical):
- **Fire** when `used >= threshold`, the reading is fresh (observed within 10 minutes), the
  window is not elapsed, and `AlertMemory` has no entry for `(meter, threshold)`.
- If both thresholds are crossed by the same reading, fire **only critical** and record both.
- **Re-arm** (delete the entry) when `used < threshold - 5 points`, or when the recorded
  `window_resets_at` has passed. This is robust whether or not a provider's `resetsAt` is
  stable between reads.
- `AlertMemory` is persisted, so a restart never repeats a notification.
- Nothing fires during the first-run backfill or for `Stale`/`Elapsed` data.
Result: at most one notification per threshold per window instance.

### 7.7 Caching, errors, staleness
- The latest `Snapshot` lives in a `watch` channel: `get_snapshot` is a clone, never IO.
- A snapshot event is emitted only when the snapshot differs from the previous one,
  ignoring `generated_at`.
- A failure never erases a good reading: `Known` → `Stale{Failing(reason)}`. Freshness per
  source: gateway log 10 min; interval sources 2.5 x their current interval.
- Startup: every provider is `Pending` until its first source answers or fails; the gateway
  log answers in milliseconds, Codex in about 3 s.

### 7.8 Time
- All instants are UTC epoch integers end to end. Core never asks for the local zone. The
  only Rust code that needs local time is `text` (tray and notification strings); it takes
  the conversion as an argument:
  ```rust
  pub struct LocalStamp { pub year: i32, pub month: u8, pub day: u8, pub weekday: u8, pub hour: u8, pub minute: u8 }
  pub trait LocalTime: Send + Sync { fn local(&self, at: UnixSeconds) -> LocalStamp; }
  ```
  The shell implements it with `chrono` (`clock` feature only: pure-Rust reading of the
  system zone, re-read on each call so a zone change is picked up). Tests pass a fixed zone.
- The UI formats with `Intl.DateTimeFormat` in the system zone, so a timezone change is
  picked up at the next render.
- Tests use `FakeClock` (settable, advanceable) and tokio's paused time; no test sleeps.

## 8. Security and privacy model

### 8.1 Assets
1. Prompt text and prompt-adjacent metadata in the gateway's files (`gateway.jsonl`,
   `turns.jsonl`, `code-turns.jsonl`, `history`, `sessions/`, and most keys of `usage.jsonl`).
2. Secrets: `DEEPSEEK_API_KEY` and every other key in `~/.jev-router.env`; the OAuth tokens
   of Claude, Codex, Gemini, Grok in their own homes.
3. Identity: account e-mail, user/account/installation ids, the home directory name.
4. Integrity of the gateway's files and of the user's quota (the app must not spend it).

### 8.2 Threats and answers
| threat | answer |
|---|---|
| A bug copies a prompt or key into the UI, a log, the store or a notification | The domain has no type that can hold one (4.1: only `Label`). Parsers are allow-lists. Canary tests (8.9). |
| Hostile content in a log line or a provider response (injection into the web view) | Only numbers, enums and `Label`s reach the UI. Svelte escapes text. No `{@html}`, no `innerHTML` (lint rule). Strict CSP, no inline or remote script. |
| The web view is compromised anyway | It can call six commands that return numbers. No shell, fs, http, opener, clipboard, dialog or updater plugin is compiled in. Capabilities grant event-listen only. |
| A dependency is malicious or vulnerable | Section 8.8. |
| Another local process of the same user reads our files or memory | Out of scope in general (same-uid processes can read `~/.jev-router.env` directly). We still write 0600/0700, disable core dumps and ptrace-attach (`PR_SET_DUMPABLE 0`), and keep secrets out of files. |
| The app spends quota or disturbs the gateway | Read-only on gateway files; no request through the gateway; the single spending path (`ClaudeLive`) is opt-in, manual, labelled. |
| A lazy installer shim runs (mise wrappers for `grok`, `agy`) | 8.6. |
| Secrets inherited by a child process | Children get a scrubbed, allow-listed environment (8.6). |
| Network exfiltration or tracking | One HTTP client, one allowed host, HTTPS only, no redirects, no proxy, no telemetry, no updater; the web view has `connect-src` limited to IPC. |

### 8.3 What is never read
- `gateway.jsonl`, `turns.jsonl`, `code-turns.jsonl`, `orch*.jsonl`, `history`, `sessions/`,
  `gateway.out`: **never opened**. An architecture test fails if any of these file names
  appears in dashboard source outside the deny-list test itself.
- In `usage.jsonl`: only `ts_ms` and `ratelimit` keys beginning
  `anthropic-ratelimit-unified-`. Honest limit: reading a file necessarily brings its bytes
  through a private read buffer and the JSON tokenizer. Those buffers are reused, never
  logged, never copied out, and zeroized when the tail reader is dropped. Nothing but the
  parsed numbers leaves `parse_line`.
- OAuth/token files of any CLI (`~/.claude*`, `~/.codex`, `~/.gemini`, `~/.grok`): never
  opened by dashboard code. (The engine's existing probe checks that some of them exist; it
  does not read tokens.)
- `claude auth status` output is read only by the engine's existing parser, which extracts
  the logged-in flag and the plan and drops the rest. The dashboard never sees the text.
- Codex `account/read` is never called.
- From `~/.jev-router.env`: only the `DEEPSEEK_API_KEY` line's value is kept; the file text
  lives in a zeroizing buffer for the duration of the parse.

### 8.4 What is never stored, logged or sent
- **Stored** (history.db, settings.json): numbers, booleans, closed-vocabulary keys. Nothing
  else. No paths, no timestamps finer than a second for samples.
- **Logged**: the app has one diagnostic channel, `diag::emit(DiagEvent)`, where `DiagEvent`
  is an enum whose fields are numbers and closed enums. There is no API to log a string.
  `println!`, `eprintln!`, `dbg!`, `log`/`tracing` macros are denied by lint in the three
  crates (`clippy::print_stdout`, `print_stderr`, `dbg_macro`, `disallowed_macros`).
  Output goes to stderr only; nothing is written to disk by default.
- **Errors**: every error type is a field-less or numeric enum. No `anyhow`, no
  `Box<dyn Error>`, no `to_string()` of a foreign error crosses a module boundary
  (foreign errors are classified at the adapter and dropped).
- **Panics**: `panic = "abort"` in `dist`; the panic hook prints only `file:line`, never the
  payload. No crash reporter, no upload. Core dumps disabled.
- **IPC**: section 7.4 is exhaustive.
- **Notifications and tray**: provider name, window name, percentage, reset time. Built only
  by `core::text` from typed inputs.
- **Network**: only what 8.7 lists.

### 8.5 Secrets in memory
```rust
// jev-usage-core::secret
/// Holds a secret. Deliberately implements NONE of: Debug, Display, Clone, Serialize,
/// PartialEq, Hash. A struct that contains one cannot derive them either, so a secret
/// cannot be printed, logged, compared, cloned or serialized by accident: it does not compile.
pub struct Secret<T: Zeroize>(T);
impl<T: Zeroize> Secret<T> {
    pub fn new(value: T) -> Self;
    /// The single way out. Call sites are counted by a test (exactly one: the DeepSeek request).
    pub fn expose_secret(&self) -> &T;
}
impl<T: Zeroize> Drop for Secret<T> { /* zeroize */ }
```
- The key is read at fetch time, used for one request, and dropped (zeroized) when the fetch
  ends. It is not cached between fetches and never placed in a struct that outlives the call.
- It is read by `io::secrets::deepseek_key() -> Option<Secret<String>>`: process environment
  first, else the env file read into a `Zeroizing<String>` and parsed with the engine's pure
  `embed::parse_env` (DRY: same parsing rules as the engine).
- The header value is marked sensitive (`HeaderValue::set_sensitive(true)`), so the HTTP
  stack will not print it.
- Honest limits: the allocator may have left copies in freed memory before zeroization of
  intermediate buffers we do not own (the engine's parser returns a `String` we move, not
  copy; `std::env::var` returns a copy we own and zeroize). The TLS stack holds the header
  bytes until the request is written. Swap is outside our control.

### 8.6 Subprocesses
One module, `io::proc`, is the only place `std::process`/`tokio::process` may be used
(architecture test). Rules:
- Program names come from a closed enum (`Program::Codex`, `Program::Claude`). There is no
  variant for `grok`, `agy`, `gemini` or `mise`, so they cannot be run.
- A program is spawned only if the engine's probe reports its provider `Ready` (the probe
  decides from files and from `claude auth status`/`codex login status`, and never runs the
  lazy shims).
- Arguments are constant arrays. No shell, no string interpolation.
- Environment: `env_clear()` then exactly `HOME`, `PATH`, `LANG`, `LC_ALL`, `XDG_CONFIG_HOME`,
  `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR` if set, `TERM=dumb`.
  No `*_API_KEY` ever reaches a child. For `ClaudeLive`, `ANTHROPIC_BASE_URL` is not set, so
  it does not go through the gateway.
- Working directory: `<state_dir>/dashboard/run` (empty, 0700), so no project config is read.
- stdin/stdout piped, **stderr discarded**. Output caps (1 MiB per line, 4 MiB total).
  Deadline with kill of the process group; `kill_on_drop`.
- Child output is parsed by an allow-list parser and never logged or stored raw.

### 8.7 Network
- One module, `io::http`, is the only place `reqwest` may be used (architecture test). It
  exposes `get_json(endpoint: Endpoint, bearer: &Secret<String>) -> Result<Vec<u8>, Unavailable>`
  where `Endpoint` is a closed enum (`DeepSeekBalance`) mapped to a `const` URL.
- Client: rustls, `https_only(true)`, `redirect(Policy::none())`, `no_proxy()`, connect
  timeout 5 s, total timeout 10 s, body cap 64 KiB, no cookies, a fixed generic `User-Agent`
  (`jev-dashboard`), no other headers.
- Egress inventory (complete): (1) `api.deepseek.com` by this client, only when a key
  exists; (2) the `codex` child talks to OpenAI with its own login; (3) the opt-in `claude`
  child talks to Anthropic with its own login. Nothing else: no updater, no telemetry, no
  fonts or assets fetched at runtime, no link opening.

### 8.8 Tauri hardening
- Plugins compiled in: `tauri-plugin-single-instance` and `tauri-plugin-notification`
  (called from Rust only). **Not** compiled in: shell, opener, fs, http, dialog, clipboard,
  updater, process, os, global-shortcut, window-state, autostart.
- Commands are declared in `build.rs` through `AppManifest::commands(&[...])`, so each needs
  an explicit permission. The single capability (`capabilities/main.json`), window `main`,
  local only:
  ```json
  { "identifier": "main", "windows": ["main"], "local": true,
    "permissions": ["core:event:allow-listen", "core:event:allow-unlisten",
                    "allow-get-snapshot", "allow-refresh", "allow-get-history",
                    "allow-get-settings", "allow-update-settings", "allow-get-diagnostics"] }
  ```
  No `core:default`, no window, webview, path, menu, tray, image or resources permissions.
- `tauri.conf.json`: `app.withGlobalTauri: false`; `app.security.freezePrototype: true`;
  `app.security.dangerousDisableAssetCspModification: false`; no `assetProtocol`; no remote
  URLs in `capabilities`; CSP:
  ```
  default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:;
  font-src 'self'; connect-src ipc: http://ipc.localhost; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'; object-src 'none'
  ```
  No `'unsafe-inline'`, no `'unsafe-eval'`. The UI therefore sets dynamic styles through CSS
  custom properties via the CSSOM and uses CSS animations, never injected `<style>` or
  `style=""` strings.
- Navigation is locked: `on_navigation` allows only the app origin; new windows are denied.
- Devtools: the `devtools` Cargo feature is not enabled; a test asserts the release manifest.
- The context menu and reload shortcuts are disabled in release.
- At start (`hardening::apply`): `umask(0o077)`, `PR_SET_DUMPABLE=0`, the panic hook. Uses
  safe wrappers from `rustix`; `unsafe` is forbidden in core and io and denied in the shell.

### 8.9 Dependency and supply-chain policy
- Rust: lockfile committed, every command `--locked`. `deny.toml`: advisories = deny
  (vulnerabilities, unsound, yanked), sources = crates.io only (no git), licenses = explicit
  allow-list, bans = `openssl`, `openssl-sys`, `native-tls` denied, wildcard versions denied.
  `cargo audit` as a second advisory source.
- Direct Rust dependencies, complete list (additions need lead approval):
  core: `serde`, `serde_json`, `zeroize`, `ts-rs` (dev/bindings feature); dev: `proptest`,
  `static_assertions`. io: `tokio`, `notify`, `rusqlite` (bundled), `reqwest` (rustls),
  `rustix`, `jev-router`; dev: `tempfile`, `proptest`. shell: `tauri` (features: `tray-icon`
  only), `tauri-build`, `tauri-plugin-single-instance`, `tauri-plugin-notification`,
  `chrono` (no default features, `clock`), `rustix`.
- JavaScript: `pnpm-lock.yaml` committed, `--frozen-lockfile`; exact versions
  (`save-exact=true`); `.npmrc`/workspace settings: dependency build scripts blocked except an
  explicit allow-list (`esbuild`), `minimumReleaseAge` of 7 days for new versions;
  `pnpm audit --prod` must be clean. Runtime dependencies: `svelte`, `@tauri-apps/api`, two
  font packages (assets only). Everything else is a dev dependency.
- No CDN, no remote font, no remote script: everything is bundled and served from the app.
- Build tools fetched at install time (`@tauri-apps/cli`) are pinned in the lockfile.

### 8.10 Tests that prove the rules
| rule | proof |
|---|---|
| No prompt can surface from the log parser | **Canary fixture**: `dashboard/fixtures/claude-gateway/canary.jsonl` has the canary string `JEVCANARY_9f3c1b7e_DO_NOT_SURFACE` in every non-allow-listed key, as extra keys, inside allow-listed keys' values, and as a window name. Test: `format!("{:?}")` and the JSON of everything `parse_line` returns contain no `JEVCANARY`. Property test: for arbitrary JSON rows with the canary at arbitrary depth, the same holds. |
| No secret or identity crosses any adapter | Every adapter test feeds canary-laced input (Codex `account_read.response.json` with `canary-email@example.invalid`, a DeepSeek response with the canary as an extra field, the canary as the DeepSeek key) and asserts the canary is absent from the returned `Reading`/error Debug output. |
| End to end: nothing leaks to IPC, store, settings, logs | `io` integration test: a service wired with canary-fed sources and a temp state dir runs a full cycle; then every emitted `Snapshot` (JSON), every file under the temp dir (raw bytes, including SQLite WAL), and captured `diag` output are searched for `JEVCANARY`, the canary e-mail and the canary key. |
| IPC carries only allowed fields | Test serialises generated values (proptest strategies for every IPC type) and walks the JSON: every key must be in `ipc-allowlist.txt`; no key matches `(?i)email|token|secret|password|prompt|message|content|account|session|cwd|path|url|auth|key$`; every string leaf must match a closed-vocabulary set or `^[a-z0-9_:.+-]{1,48}$`. The same walk runs over the generated `.ts` bindings' property names. |
| `Secret` cannot be printed/cloned/serialised | `static_assertions::assert_not_impl_any!(Secret<String>: Debug, Display, Clone, Serialize, PartialEq)`. |
| One exit for secrets, one HTTP client, one process spawner | Architecture test (plain Rust test that scans `dashboard/**/*.rs`): `expose_secret(` only in `io/src/sources/deepseek.rs`; `reqwest` only in `io/src/http.rs`; `process::Command` only in `io/src/proc.rs`; `jev_router::` only as `jev_router::embed`; forbidden file names (8.3) nowhere; no `println!`/`eprintln!`/`dbg!`. |
| Children get no secrets | Test spawns a stub program through `proc` with `DEEPSEEK_API_KEY=JEVCANARY...` in the parent environment; the stub prints its environment; assert no canary and only allow-listed names. |
| Files are private | Test asserts 0700 on the directory and 0600 on `history.db`, `-wal`, `-shm`, `settings.json`. |
| Capabilities and CSP do not drift | Snapshot tests of `capabilities/main.json`, the CSP string, the plugin list in `Cargo.toml`, and "no `devtools` feature". |
| The web view makes no network request | Playwright test fails on any request that is not the app origin or IPC; CSP violation events fail the test. |
| No raw HTML sinks | ESLint rule bans `{@html}`, `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `eval`, `new Function`. |

## 9. UI/UX specification

### 9.1 Design intent
An instrument, not a marketing page: calm, dense, exact. One screen answers the question;
numbers are the heroes; colour is reserved for status. Nothing moves unless a value changed.
Workers implement this spec with the `frontend-design` and `impeccable` skills and finish
with a `better-ui` pass; deviations from tokens or wireframes need lead approval.

### 9.2 Information architecture
```
Overview (default)  ─┬─ Provider detail (one per provider)
                     └─ Settings ── Thresholds / Notifications / Appearance / Behaviour / Providers / Diagnostics
Tray icon + menu (always)        Notifications (threshold crossings only)
```
Three views, an in-memory router (no URL bar, no history plugin), `Esc` goes back.

### 9.3 Overview
One card per configured provider, ordered by severity (critical, warn, ok) then by fixed
provider order; ties never reorder on their own. Providers without a usage interface are one
quiet line at the bottom.

```
┌──────────────────────────────────────────────────────────────┐
│ Usage                                   Updated 12:33  ⟳  ⚙  │  header 48
├──────────────────────────────────────────────────────────────┤
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ● Claude                                      Max 5x   › │ │  card header
│ │                                                          │ │
│ │ 5-hour            50%   ████████████░░░░░░░░░░░░░░       │ │  meter row
│ │                   resets in 2h 14m · today 14:00         │ │
│ │ 7-day             37%   █████████░░░░░░░░░░░░░░░░░       │ │
│ │                   resets in 2d 13h · Thu 02:00           │ │
│ │                                         as of 12:33      │ │  freshness
│ └──────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ● Codex                                         Free   › │ │
│ │ 30-day             4%   █░░░░░░░░░░░░░░░░░░░░░░░░░       │ │
│ │                   resets in 29d 11h · 16 Dec 08:37       │ │
│ └──────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ○ DeepSeek                                               │ │
│ │ Add a key to see your balance.                           │ │
│ │ DEEPSEEK_API_KEY=…  in ~/.jev-router.env        [Copy]   │ │
│ └──────────────────────────────────────────────────────────┘ │
│  No usage interface yet: Gemini · Antigravity · Grok     ⓘ   │
└──────────────────────────────────────────────────────────────┘
```
- The percentage is the largest element in a row (type `num-lg`), right-aligned in a fixed
  column so digits line up across cards (tabular numerals).
- The bar is 8 px high, full radius, with tick marks at the warn and critical thresholds.
- Layout is fluid (a tiling compositor decides the size): one column below 640 px, two
  columns from 640 px, three from 1100 px; card max width 520 px; page padding 16 px (24 px
  from 640 px). Minimum supported window 320 x 400.
- The whole card is one button (to the detail view) with the chevron as affordance; inner
  controls (Copy, Refresh) are separate tab stops.
- Balance meters (DeepSeek) show the amount as the hero (`¥110.00`) with "granted ¥10.00 ·
  topped up ¥100.00" as the secondary line, and no bar.

### 9.4 Provider detail
```
┌──────────────────────────────────────────────────────────────┐
│ ‹ Claude                          Max 5x      as of 12:33  ⟳ │
├──────────────────────────────────────────────────────────────┤
│  5-hour                                                      │
│  50%          resets in 2h 14m                               │
│  ████████████░░░░░░░░░░░░   today 14:00 · status: allowed    │
│                                                              │
│  7-day                                                       │
│  37%          resets in 2d 13h                               │
│  █████████░░░░░░░░░░░░░░░   Thu 02:00 · status: allowed      │
├──────────────────────────────────────────────────────────────┤
│  History      [ 5-hour ▾ ]              ( Day  Week  Month  Quarter )
│ 100%┤                                                        │
│  95%┤- - - - - - - - - - - - - - - - - - - - - - - critical  │
│  80%┤- - - - - - - - - - - - - - - - - - - - - - - warn      │
│     │            ╭──╮                                        │
│  50%┤        ╭───╯  │        ╭────╮                  ╭──     │
│     │   ╭────╯      │   ╭────╯    │             ╭────╯       │
│   0%┼───╯           ╰───╯         ╰─────────────╯            │
│     └──────┬───────────┬───────────┬───────────┬──────       │
│          06:00       12:00       18:00       00:00           │
│  Peak 72% at 09:40 · 3 resets in this range                  │
├──────────────────────────────────────────────────────────────┤
│  Source: gateway log (updates when you use jev-claude)       │
│  [ Check now ]  uses one small request          (if enabled) │
└──────────────────────────────────────────────────────────────┘
```
- Chart: a **step** line (usage is piecewise constant between samples) with a soft area
  fill, vertical hairlines at window resets, dashed threshold lines labelled in text, gaps
  drawn as gaps (never interpolated across missing data). Rollup ranges draw the `lo..hi`
  band. Hand-written SVG, no chart library.
- Chart accessibility: `role="img"` with a generated summary ("5-hour usage over the last
  day: peak 72 % at 09:40, now 50 %, 3 resets"); a visually hidden data table of the points;
  keyboard: focus the plot, Left/Right move a crosshair between points, the value is
  announced through a polite live region. Pointer: hover or drag shows the same readout.
- Range control is a radio group (arrow keys). The meter selector appears only when the
  provider has more than one meter.
- Balance history uses the same chart with a currency axis and no thresholds.

### 9.5 States (every card and the detail header)
```
PENDING            │ ● Codex                                │  skeleton rows, no shimmer under
                   │ ▒▒▒▒▒▒▒   ▒▒▒   ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒      │  reduced motion; shown ≥ 300 ms
                   │ Checking…                              │  only if the fetch takes > 150 ms

STALE (quiet)      │ ● Claude                      Max 5x   │  values keep full contrast;
                   │ 5-hour   50%  ████████░░░░░░           │  the freshness line turns into
                   │          resets in 12m · today 14:00   │  a labelled note with a clock icon
                   │ ◷ Last seen 2h 05m ago · updates when you use jev-claude │

ELAPSED window     │ 5-hour    —   ░░░░░░░░░░░░░░           │  no number is invented
                   │          Reset at 14:00 · waiting for your next request   │

STALE (failing)    │ ● Codex                         Free   │
                   │ 30-day    4%  █░░░░░░░░░░░░░           │
                   │ ⚠ Can't reach Codex · showing 12:31 · retrying in 4m  [Retry] │

UNAVAILABLE        │ ▲ Codex                                │  no reading at all
                   │ Not signed in to Codex.                │
                   │ codex login                    [Copy]  │

NOT CONFIGURED     │ ○ Claude                               │
                   │ No usage seen yet.                     │
                   │ Run jev-claude once and it appears here.   jev-claude  [Copy] │

OFFLINE (all net)  banner under the header:  ⚠ You're offline · showing saved values
                   (Claude's card is unaffected: it reads a local file)

FIRST RUN          centred, once:   Usage, in one place.
                   Claude and Codex appear automatically when they're set up.
                   [ ✓ Claude · found ]  [ ✓ Codex · found ]  [ ○ DeepSeek · add a key ]
                   then the normal overview replaces it as soon as one provider answers.
```
Copy for every `Unavailable` and `Setup` value lives in one table (`src/lib/copy.ts`):
a one-sentence cause in plain words, and when there is one, the exact command to copy.
No error codes, no stack traces, no "Something went wrong".

### 9.6 Settings
```
┌──────────────────────────────────────────────────────────────┐
│ ‹ Settings                                                   │
├──────────────────────────────────────────────────────────────┤
│ Thresholds                                                   │
│   Warn at        [ 80 ] %      ├────────●───┤                │
│   Critical at    [ 95 ] %      ├──────────●─┤                │
│ Notifications                                                │
│   [x] When a window passes the warn level                    │
│   [x] When a window passes the critical level                │
│   Sent once per window. Never repeated.                      │
│ Appearance                                                   │
│   Theme   (•) System  ( ) Light  ( ) Dark                    │
│   Clock   (•) Auto    ( ) 12-hour ( ) 24-hour                │
│ Behaviour                                                    │
│   [x] Keep running in the tray when the window closes        │
│   [ ] Start at login (hidden)                                │
│ Providers                                                    │
│   [x] Claude   [x] Codex   [x] DeepSeek                      │
│   [ ] Claude live check  · uses one small request per check  │
│ Diagnostics                                            ▸     │
│   Version 0.1.0 · history 1.2 MB, 8 412 samples              │
│   claude gateway log   ok 12:33                              │
│   codex app-server     ok 12:31                              │
└──────────────────────────────────────────────────────────────┘
```
Settings apply immediately (no Save button); each control shows a brief inline "Saved" tick;
invalid threshold input is corrected on blur with an explanation (warn must be below
critical; range 50..100). Turning on "Claude live check" opens a confirmation dialog.
"Keep running in the tray" is disabled with an explanation when no tray host exists.

### 9.7 Design system (tokens)
Tokens are defined once in `src/lib/design/tokens.css` as CSS custom properties. Components
use tokens only (a stylelint rule bans raw colours, raw font sizes and raw spacing in
component styles).

**Type.** Geist Variable for text, Geist Mono Variable for every number, time and command
(both OFL, self-hosted, latin subset, `font-display: block` since they are local). Fallback
`system-ui`, `ui-monospace`. All numerals `font-variant-numeric: tabular-nums`.

| token | size / line | weight | use |
|---|---|---|---|
| `text-xs` | 12 / 16 | 450 | captions, axis labels |
| `text-sm` | 13 / 18 | 450 | secondary lines |
| `text-md` | 14 / 20 | 450 | body, controls |
| `text-lg` | 16 / 22 | 550 | card titles |
| `text-xl` | 20 / 26 | 600 | view titles |
| `num-lg` | 28 / 32 mono | 550 | meter percentage on cards |
| `num-xl` | 40 / 44 mono | 550 | hero number in detail |

**Space** (4 px base): `1`=4, `2`=8, `3`=12, `4`=16, `6`=24, `8`=32, `12`=48.
**Radius**: `sm` 6, `md` 10, `lg` 14, `full`. Nested radii are concentric (inner = outer −
padding). **Borders**: 1 px hairlines; elevation is a border plus a surface step, no drop
shadows except the focus ring and dialogs. **Hit targets**: at least 32 x 32 px with pointer,
controls at least 28 px high.

**Colour** (OKLCH; light / dark). Neutrals carry a slight cool tint so status hues stay clean.

| token | light | dark | use |
|---|---|---|---|
| `bg` | `oklch(98.5% 0.003 250)` | `oklch(16% 0.008 250)` | window |
| `surface` | `oklch(100% 0 0)` | `oklch(20.5% 0.009 250)` | cards |
| `surface-2` | `oklch(96.5% 0.004 250)` | `oklch(24.5% 0.010 250)` | wells, bar track |
| `border` | `oklch(90% 0.006 250)` | `oklch(30% 0.011 250)` | hairlines |
| `border-strong` | `oklch(60% 0.010 250)` | `oklch(58% 0.012 250)` | control outlines (3:1) |
| `text` | `oklch(21% 0.012 250)` | `oklch(95% 0.005 250)` | primary |
| `text-2` | `oklch(44% 0.012 250)` | `oklch(74% 0.008 250)` | secondary (AA) |
| `accent` | `oklch(52% 0.16 255)` | `oklch(74% 0.13 255)` | focus, links, selected |
| `ok` | `oklch(52% 0.11 235)` | `oklch(76% 0.10 230)` | below warn (blue, not green) |
| `warn` | `oklch(56% 0.13 70)` | `oklch(80% 0.13 80)` | warn level (amber) |
| `critical` | `oklch(52% 0.19 30)` | `oklch(72% 0.17 32)` | critical level (vermilion) |
| `unknown` | `oklch(58% 0.006 250)` | `oklch(62% 0.006 250)` | no data |

Status is **never colour alone**: each level also has a shape and a word.

| level | hue | glyph | bar | word |
|---|---|---|---|---|
| ok | blue | ● filled circle | solid | none |
| warn | amber | ▲ triangle | solid + threshold tick reached | "High" |
| critical | vermilion | ■ square | diagonal hatch | "Almost out" / "Limit reached" |
| unknown | grey | ○ ring | empty track, dashed outline | "No data" |

The blue/amber/vermilion triple (Okabe-Ito family) stays distinguishable under protanopia,
deuteranopia and tritanopia; shapes and words cover monochromacy. Contrast is tested from the
token file: text ≥ 4.5:1 (target 7:1 for `text`), large numerals ≥ 4.5:1, UI boundaries and
status glyphs ≥ 3:1, in both themes.

**Theme** follows the OS (`prefers-color-scheme`) with a Settings override applied as
`data-theme` on `<html>` before first paint (no flash). `color-scheme` is set so native
controls match. `forced-colors: active` is supported (system colours, borders instead of
fills).

**Motion.** Durations `fast` 120 ms, `base` 200 ms, `slow` 320 ms; easing
`cubic-bezier(0.2, 0, 0, 1)`; only `transform`, `opacity` and the bar's scale animate.
A bar animates to a new value (`slow`); a changed number cross-fades (`fast`); views
cross-fade (`fast`), no slides. Nothing loops. `prefers-reduced-motion: reduce` removes all
movement and keeps at most a 100 ms opacity change; skeletons become static.

### 9.8 Micro-interactions
- Refresh (⟳): the icon rotates once per fetch (static under reduced motion), the button is
  `aria-busy`; "Updated 12:33" updates in place and is announced politely. A second click
  while busy does nothing.
- Copy: the button label becomes "Copied" with a tick for 1.5 s and is announced.
- Threshold change: the tick marks on every bar move live while the slider moves.
- A value that crosses a threshold changes glyph, hue and word together, once, with no
  flashing or shaking.
- Hover on a bar shows exact figures in a tooltip that is also reachable by focus
  ("50.0 % used · window ends today 14:00 · status allowed").
- Card press: 1 px translate and a `surface-2` flash (`fast`).

### 9.9 Numbers, time and copy
Rules (one implementation per language, shared vectors in `format-vectors.json`):
- **Percent**: whole numbers. `0 < x < 1` → "<1%". `99.5 ≤ x < 100` → "99%". 100 and above →
  the real figure ("100%", "104%"). Never round up to 100.
- **Countdown** (to `resets_at`, from now): ≥ 48 h "3d 4h"; ≥ 1 h "2h 14m"; ≥ 1 min "38m";
  below "under a minute"; past → the `Elapsed` state, never a negative.
- **Absolute time**, local zone, hour cycle from the setting: same day "today 14:00";
  next day "tomorrow 02:00"; within 6 days "Thu 02:00"; else "16 Dec 08:37" (year added when
  it differs). Always shown next to the countdown, never one without the other.
- **Age**: "just now" (< 1 min), "5m ago", "2h 05m ago", "yesterday 21:10", then a date.
- **Money**: `Intl.NumberFormat` currency style, two decimals, unknown currency as a plain
  number with its code.
- **Window names**: "5-hour", "7-day", "30-day", "N-hour"/"N-day" for rolling windows,
  the raw label in mono for named windows.
- **Tone**: plain, specific, calm. Sentence case. State the fact, then the next step.
  "Not signed in to Codex." not "Authentication error". No exclamation marks, no blame, no
  jargon ("rate limit window" is "window"; "utilization" is "used").

### 9.10 Accessibility
- WCAG 2.2 AA minimum, AAA contrast for primary text.
- Every function by keyboard. Tab order follows reading order. Shortcuts: `R` refresh,
  `,` settings, `Esc` back, `1`–`6` open provider, `?` shortcut list. Shortcuts are single
  keys only when no text field has focus and can be turned off in Settings.
- Focus: 2 px `accent` ring with 2 px offset on `:focus-visible`, never removed; focus moves
  to the view title on navigation and returns to the triggering control on back.
- Semantics: one `<main>`, a heading per view and per card; each meter is
  `role="meter"` with `aria-valuenow/min/max` and `aria-valuetext`
  ("50 percent used, resets in 2 hours 14 minutes, today at 14:00"); status changes go to one
  polite live region; errors that need action use `role="alert"` once.
- Zoom to 200 % and a 320 px wide window without loss; text respects the system font scale.
- Automated: axe on every component and screen state; manual: a keyboard walk-through and an
  Orca read-through are part of the definition of done.

### 9.11 Tray
- Icon: a ring gauge drawn at run time (pure function, RGBA, 64 px, cached per 5 %-step and
  level). Fill arc = the headline usage. Level is also a shape: ok = ring only; warn = ring
  plus a centre dot; critical = ring plus a filled centre; no data = dashed ring. A mid-grey
  outline keeps it legible on light and dark panels.
- Menu (Linux tray hosts deliver the menu, not click events, so everything is in the menu):
  ```
  Claude · 5-hour 50% · resets 14:00          (information rows, not clickable)
  Claude · 7-day 37% · resets Thu 02:00
  Codex · 30-day 4% · resets 16 Dec
  ─────────────────────────────
  Open dashboard
  Refresh now
  [x] Notifications
  ─────────────────────────────
  Quit
  ```
  Rows use absolute reset times so the menu never needs a timer. The menu is rebuilt only
  when the snapshot changes.
- No tray host on the bus: the app says so in Settings, closing the window quits, and a
  second launch focuses the existing window (single-instance).

### 9.12 Notifications
- Only threshold crossings (section 7.6). Title "Claude 5-hour at 80%"; body "Resets in
  2h 14m (today 14:00)." Critical uses the critical urgency hint. No sound request.
- Never: repeated alerts for the same window, alerts for stale data, alerts at start-up for
  a level that was already announced, marketing, tips, "back to normal" messages.

### 9.13 Window behaviour (Hyprland/Wayland)
- The app is resident in the tray. **The web view exists only while the window is open**:
  closing the window hides it and, after 30 s, destroys the web view; opening recreates it
  (target under 400 ms). Hidden cost is therefore the Rust core only.
- Default size 420 x 640, minimum 320 x 400; the last size is saved in `settings.json`
  (debounced) and restored. Position is the compositor's business on Wayland and is not
  stored. The layout is fluid because a tiling compositor may give any size.
- `jev-dashboard --hidden` starts in the tray (used by autostart). A second launch focuses
  the window.
- Autostart writes or removes `~/.config/autostart/dev.jev.dashboard.desktop` (pure text
  generation, atomic write). Documented alternative for Hyprland without XDG autostart:
  `exec-once = jev-dashboard --hidden`. The app never edits Hyprland's configuration; the
  README offers an optional `windowrule` snippet (float, size) for users who want a floating
  panel.
- Server-side decorations are left to the compositor; there is no custom title bar.

## 10. Frontend stack

| choice | decision | why |
|---|---|---|
| Framework | **Svelte 5** (runes) + TypeScript strict | Compiles to small direct DOM code, about a third of a React bundle here; fine-grained updates suit a few live numbers; scoped styles; no runtime CSP issues. |
| Build | Vite, static output (no SSR, no router package) | Tauri serves static files. |
| State | One `usage` store (a rune-based class) fed by `get_snapshot` + the `snapshot` event; `settings` store; a tiny in-memory router | The backend owns the state; the UI holds a read model. No state library. |
| IPC | `@tauri-apps/api` `invoke`/`listen`, wrapped once in `src/lib/ipc/client.ts` behind an `IpcClient` interface with a mock implementation for tests | One seam; components never import Tauri. |
| Chart | Hand-written SVG component (`Chart.svelte`, about 250 lines) | Full control of accessibility, theming, step rendering and gaps; no dependency. |
| Styles | Plain CSS with custom properties, scoped per component | No CSS framework, no runtime. |
| Icons | Inline SVG sprites (about 12 icons, own set) | No icon package. |
| Fonts | `@fontsource-variable/geist` and `geist-mono`, latin subset | Local, consistent numerals. |

Budgets (checked in CI from the build output): JavaScript ≤ 70 KB gzip, CSS ≤ 12 KB gzip,
fonts ≤ 120 KB, no request other than the app origin and IPC at run time, first paint with
data ≤ 400 ms after window creation on this machine.

Frontend tests:
- **Unit** (Vitest): `format`, `copy`, stores, router, chart geometry (pure functions),
  driven by `format-vectors.json`.
- **Component** (Vitest + Testing Library + axe): every component in every state.
- **Screen and visual** (Playwright against the built UI with the mock `IpcClient`): each
  screen x state x theme x width (360, 720, 1200) screenshot compared to a committed
  baseline; axe on each; reduced-motion and forced-colors runs; "no network" assertion.
- **End to end on the real build**: `tauri-driver` + `WebKitWebDriver` when available
  (section 11.4), driving the release binary started with a temp state dir and fixture
  sources.

## 11. Quality gates

"Zero bugs" cannot be promised by anyone. What can be promised is that every known class of
defect has a mechanical check, that the dangerous classes are unrepresentable in the types,
and that nothing merges without the gates below and a lead review.

### 11.1 Static
- Rust: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` with workspace
  lints: `clippy::all`, `pedantic` (selected allow-list written in the manifest),
  `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `arithmetic_side_effects`,
  `as_conversions`, `print_stdout`, `print_stderr`, `dbg_macro`, `todo`, `unimplemented`
  denied outside tests; `unsafe_code` forbidden (core, io) / denied (shell);
  `missing_docs` warned on public items.
- TypeScript: `tsc --noEmit` strict (`noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`),
  `svelte-check`, ESLint (security rules of 8.10), stylelint (token rule), Prettier check.
- Generated bindings up to date (`git diff --exit-code` after regeneration).

### 11.2 Tests
| level | what | tool |
|---|---|---|
| unit | every pure function in core; every formatter | `cargo test`, Vitest |
| property | parsers never panic and round-trip; `Percent`/`Money`/`MeterId::key` laws; `alerts::step` invariants (≤ 1 alert per threshold per window instance for any reading sequence); `assess::derive` totality; `should_store`/`downsample` bounds; tail reader equals whole-file read for any chunking and any rotation point | `proptest` (default 256 cases; CI 4096) |
| fuzz | `parse::claude_log`, `parse::claude_event`, `parse::codex`, `parse::deepseek`, `Money::parse_decimal`, `MeterId::parse_key`, settings JSON | `cargo-fuzz` targets in `dashboard/fuzz/`, 60 s each, when a nightly toolchain is present; the same entry points are always exercised by a stable byte-level proptest so the gate never silently disappears |
| adapter | each source against recorded fixtures and failure cases (timeout, garbage, truncated, huge line, non-zero exit, 401, 429, 500, offline) using a stub program and a local stub HTTP server | `cargo test` |
| integration | service loop with fake clock and fake sources: scheduling, backoff, visibility, suspend jump, alerts once, canary end to end | `cargo test` with tokio paused time |
| component / screen / visual / a11y | section 10 | Vitest, Playwright, axe |
| e2e | real binary | WebDriver (11.4) |
| security | section 8.10 | all of the above |

### 11.3 Coverage and budgets
- Line coverage (`cargo llvm-cov`): core ≥ 95 %, io ≥ 85 %; the shell is glue and is covered
  by e2e. Vitest: `src/lib` logic ≥ 95 %, components ≥ 85 %.
- Resource budgets, measured by `dashboard/scripts/measure-idle.sh` on the release build:
  hidden and idle for 10 minutes: CPU time < 0.5 s, average wakeups < 2 per minute, RSS
  ≤ 60 MB; binary ≤ 15 MB.

### 11.4 `scripts/ci.sh`
One script, fail-fast, prints a table of stages. Stages: (1) engine: `cargo build
--all-targets --locked` and `cargo test --locked` at the root (the 650 existing tests must
stay green); (2) fmt, clippy; (3) dashboard Rust tests incl. property tests at CI case
counts; (4) bindings drift; (5) `cargo deny check`, `cargo audit`; (6) `pnpm install
--frozen-lockfile`, lint, typecheck, unit and component tests, `pnpm audit --prod`;
(7) UI build and bundle budgets; (8) Playwright screens, visual, a11y, no-network;
(9) release build (`--profile dist`) and manifest snapshot tests; (10) e2e on the real
build; (11) coverage thresholds; (12) fuzz (60 s per target).
Tools that are not installed (`cargo-deny`, `cargo-audit`, `cargo-llvm-cov`) are installed by
`scripts/ci.sh --bootstrap` at pinned versions with `--locked`. Stages 10 and 12 depend on
tools that may not exist here (`WebKitWebDriver`, a nightly toolchain): they print
**SKIPPED (reason)** loudly, the summary lists them, and `scripts/ci.sh --release` treats a
skipped stage as a failure. No stage is ever skipped silently.

### 11.5 Definition of done (per task and for v1)
1. The task's acceptance tests exist and pass; `scripts/ci.sh` is green (no new SKIPPED).
2. No new direct dependency without lead approval; no lint allow without a written reason.
3. No `TODO`, `todo!`, commented-out code or dead code.
4. The task's security rules are each covered by a named test.
5. Public items documented; behaviour changes reflected in this design if they differ.
6. UI tasks: every state of every component implemented and screenshot-tested in both
   themes and three widths; axe clean; keyboard walk-through recorded in the PR notes.
7. Lead review passed against the task's checklist.
For v1 as a whole, additionally: `ci.sh --release` green on this machine, the idle budgets
measured, a manual run beside a live `jev-claude` session for one day with no gateway
interference, and an Orca read-through.

## 12. Packaging, install, versioning
- Build: `pnpm --dir dashboard install --frozen-lockfile && pnpm --dir dashboard tauri build
  --no-bundle -- --profile dist --locked`. The result is one binary,
  `target/dist/jev-dashboard`, which needs only the system's webkit2gtk-4.1, gtk3 and
  libayatana-appindicator (all present).
- `dashboard/scripts/install.sh [dest]` (default `~/.local/bin`): installs the binary as
  `jev-dashboard`, `dev.jev.dashboard.desktop` into `~/.local/share/applications`, and icons
  into `~/.local/share/icons/hicolor/{32x32,128x128,scalable}/apps`; `--uninstall` removes
  exactly those. It never touches the gateway, the engine binaries or user configuration.
- `scripts/install.sh` is **unchanged** and keeps installing only the engine tools.
- No AppImage by default (its bundler downloads tooling at build time). An Arch `PKGBUILD`
  is a follow-up.
- Versioning: SemVer, single source `dashboard/src-tauri/Cargo.toml` (`0.1.0` for v1);
  `tauri.conf.json` and the UI read it from there; `dashboard/CHANGELOG.md`. The app version
  is independent of the engine's. Store schema and settings file carry their own integer
  versions with migrations.
- Application identifier `dev.jev.dashboard`; product name "Jev Dashboard".

## 13. Performance design notes
- Idle hidden: no web view, no timers except the next source deadline, inotify blocked in the
  kernel.
- A burst of gateway writes costs one 1.5 s-coalesced tail read of the new bytes only, with a
  cheap byte pre-filter (`"ratelimit":{"a`) before JSON parsing.
- A Codex poll is one short-lived process (about 2.7 s wall, almost all waiting) every
  5/15 minutes; nothing stays resident.
- Snapshots are emitted only on change. Tray icons are cached. History queries return at
  most 600 points, downsampled in SQL plus `history::downsample`.
- One multi-thread runtime is already owned by Tauri; the service uses it and spawns no
  threads of its own beyond `spawn_blocking` for SQLite and the first-run backfill.

## 14. Risks
| risk | likelihood | mitigation |
|---|---|---|
| `codex app-server` is experimental and its shape changes | medium | allow-list parser tolerant of unknown fields; fixture + schema noted with the CLI version; failure → `Stale{Failing(Protocol)}`, never a crash; a diagnostics line shows it |
| Anthropic renames headers or adds windows | medium | prefix-based generic window parsing; unknown windows render as named meters |
| Claude usage outside the gateway makes the number low | certain | honest "as of" labelling, `Elapsed` state, optional live check |
| Tray host behaviour differs between shells | medium | menu-only interaction model; no-tray fallback; tested on this machine's host |
| WebKitGTK quirks (clipboard, fonts, compositing on Wayland) | medium | Copy falls back to selecting the text; fonts bundled; visual tests run in WebKit |
| `WebKitWebDriver` unavailable on Arch | high | Playwright-on-mock covers the UI; the Rust integration suite covers the core; real-build e2e is required for release and loudly skipped otherwise |
| The engine library drags heavy dependencies into the app | certain, low impact | link-time removal; split `jev-engine` later |
| Web view memory when open (WebKit processes, 100 MB+) | certain | web view destroyed when the window is closed |
| DeepSeek adapter unverified until a key exists | certain | built against the documented shape with fixtures; marked unverified in Diagnostics until first success |
| Shared `Cargo.lock` churn for the engine | low | `default-members`, `--locked`, engine tests in stage 1 |

## 15. Designing for the later chat window (not built)
- The engine is reached only through `jev_router::embed`, which already exports `Runtime`;
  the chat feature adds turn execution exports there and a `jev-chat-io` crate beside
  `jev-usage-io`. The usage service is unaffected.
- Turns run by the chat engine yield `PlanUsage`/rate-limit events; a `ChatTurnSource`
  implementing `UsageSource` feeds them into the same service, so Claude usage becomes live
  while chatting with no new UI.
- The IPC rule changes for chat (prompts and answers must cross). That will be a **separate
  window label with a separate capability file and separate commands**, so the dashboard
  window's "numbers only" guarantee and its tests stay intact.
- The shell's window manager already creates and destroys windows by label.

## 16. Verified versus assumed
**Verified** (section 3): Codex app-server transport, sequence, timings, exit behaviour and
free-plan shapes; `usage.jsonl` field names, value types, freshness and line size; tray and
notification hosts present; Tauri CLI `info` output; toolchain versions; which tools are
missing; `gemini --help` has no usage command; engine probes never run `grok`/`agy`.

**Assumed**: paid Codex plans report 300- and 10080-minute windows (schema only);
`account/rateLimits/updated` is not pushed without turns; meaning of Claude's `7d_oi`
window; DeepSeek balance response shape (documentation); `claude -p` emits a
`rate_limit_event` on a minimal request and its exact cheapest flags; Antigravity headless
`/usage` JSON shape; xAI Management API shape; WebKitGTK allows `navigator.clipboard.writeText`
under the app origin; XDG autostart is honoured by this session; current latest stable
versions of Tauri 2, Svelte 5 and Vite (the scaffold task pins what it finds).

## 17. Open questions
For the user:
- Q1. Product name and identifier: "Jev Dashboard" / `dev.jev.dashboard` / `jev-dashboard`?
- Q2. Claude live check: build it in v1 (off by default), and may one real request be spent
  to verify it? Otherwise it ships fixture-tested only, or is dropped.
- Q3. Start at login by default, or opt-in (design: opt-in)?
- Q4. Is an opt-in display of the account e-mail wanted at all (design: no, plan name only)?
- Q5. After v1, which comes first: Antigravity usage (needs `agy` really installed), xAI
  prepaid balance (needs a management key), or the chat window?
- Q6. The interrupted `antigravity-cli@1.5.9` download left an empty directory under
  `~/.local/share/mise/installs/antigravity-cli/`: remove it or let `agy` finish installing?

For the lead (resolved during implementation, not user-owned): split of `jev-engine` from
`jev-router`; whether to store a last reading for instant cold-start display; PKGBUILD.
