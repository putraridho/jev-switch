# Flashboard (the Jev usage dashboard): design

Status: design for v1. The user's decisions are recorded in section 1a; the questions still open are in section 17. Normative for every task in `TASKS.md`.
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
- **UI quality bar:** the UI is "minimalistic but detailed and beautiful, also a11y".
  *Minimal*: little chrome, few colours, generous whitespace, one clear visual hierarchy.
  *Detailed*: precise numbers, reset times, history and the per-window breakdown are available
  on demand through progressive disclosure (hover, focus, expand, the detail view) and are
  never hidden from the keyboard or a screen reader. *Beautiful*: considered typography, one
  spacing scale, subtle motion that respects reduced motion, polished light and dark themes.
  *Accessible*: a hard requirement, not a nice-to-have. WCAG 2.2 AA at minimum (contrast,
  visible focus, full keyboard operation, logical focus order, screen-reader names and
  live-region announcements of threshold changes that are not spammy, status never by colour
  alone, target sizes, zoom to 200 % without loss), automated accessibility checks in the test
  plan, and a manual screen-reader pass in the definition of done. Sections 9, 10 and 11.5 and
  tasks T07, T09, T10 and T13 implement this.
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
- Showing account identity (e-mail). The app shows the plan name only, and there is no
  opt-in for more (1a).
- Cost accounting per token, per project or per session. The gateway log has that data next
  to prompt-adjacent metadata; v1 deliberately does not read it.
- Windows and macOS packaging. The code stays portable; only Linux is tested and shipped.
- Auto-update, telemetry, crash upload, remote configuration. Never.
- Editing Hyprland or any other user configuration. The one file the app writes outside its
  own state directory is its own XDG autostart entry (1a, section 9.13).

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
- US8. The app starts hidden at login (I am told so on first run and can turn it off there
  or in Settings), remembers its window size, follows the OS light/dark theme, and is fully
  usable from the keyboard and a screen reader.

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
  was an integer here (0..100); the parser accepts any JSON number, because other plans are
  not recorded. `resetsAt` is epoch seconds, nullable. Whether `resetsAt` is fixed for a
  window or slides between reads is **not known**; nothing in this design depends on it.
  `rateLimitsByLimitId` can carry further limits; v1 reads `rateLimits` only.
- The spike ran with the shell's full environment and working directory. Running under the
  scrubbed environment and empty working directory of section 8.6 is **assumed** to work and
  is checked once, by hand, in T04 (this call is free and sends no prompt).
- `codex` and `claude` on this machine are mise command-wrappers (small ELF launchers in
  `~/.local/share/mise/command-wrappers/bin`) that start the installed tool.
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
- Re-checked over the last 30 MB (10 843 rows): `ratelimit` is always an object, non-empty
  on 10 528 rows.
- `ts_ms` is an integer (epoch ms), taken when the request **starts**; the row is appended
  when the request ends (`src/bin/gateway.rs`). Rows are therefore **not in `ts_ms` order**:
  about 20 % of rows are older than the row before them. The newest sample is the one with
  the greatest `ts_ms`, not the last line.
- Rotation (`src/log.rs`): before an append, a file of 50 MB or more is renamed to
  `<path>.1` (replacing the previous one); the append then creates a new file, mode 0600.
- Every `ratelimit` value is a **string**:

  | key (prefix `anthropic-ratelimit-unified-`) | example | meaning |
  |---|---|---|
  | `5h-utilization`, `7d-utilization` | `"0.5"`, `"0.37"` | fraction used, 0..1 |
  | `5h-reset`, `7d-reset` | `"1794898800"` | epoch seconds |
  | `5h-status`, `7d-status` | `"allowed"`, `"allowed_warning"` | window status |
  | `5h-surpassed-threshold` | `"0.9"` (only with `allowed_warning`) | not used in v1 |
  | `status`, `reset` | `"allowed"`, `"1794898800"` | overall; not used in v1 |
  | `fallback`, `upgrade-paths` | `"available"`, `"upgrade_plan,overage"` | not used in v1 |
  | `representative-claim` | `"five_hour"` | which window binds |
  | `fallback-percentage` | `"0.5"` | not used in v1 |
  | `overage-status`, `overage-disabled-reason` | `"rejected"`, `"org_level_disabled"` | not used in v1 |
  | `7d_oi-utilization/-reset/-status` | on 45 of 10 528 rows, never on consecutive runs | an extra window; meaning unknown; **present only on some rows** |

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
- The session is started by `uwsm`; `systemd-xdg-autostart-generator` is installed and
  `xdg-desktop-autostart.target` is active, so `~/.config/autostart/*.desktop` entries are
  started at login. The session manager's `PATH` (`systemctl --user show-environment`)
  contains the mise command-wrapper directory, so an autostarted app finds `codex` and
  `claude` on this machine.
- `rustup` is installed (a nightly toolchain can be added without root). Playwright's
  Chromium build is present in `~/.cache/ms-playwright`; its WebKit build is not, and
  Playwright does not support Arch for WebKit.
- NOT installed, installable without root by `scripts/ci.sh --bootstrap`: `cargo-deny`,
  `cargo-audit`, `cargo-llvm-cov`, `cargo-fuzz` (plus a nightly toolchain), `tauri-driver`.
- NOT installed and needing `pacman` (root, so a user decision, section 17): `WebKitWebDriver`
  (not in the Arch `webkit2gtk-4.1` package) and the `orca` screen reader. Section 11
  handles each.

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
`as_str()` and the serde/IPC name are the same string, one spelling everywhere (storage, IPC,
tests): `claude`, `codex`, `deepseek`, `gemini`, `antigravity`, `grok` (explicit
`#[serde(rename)]`, because `snake_case` would give `deep_seek`).
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
pub enum Setup { UseGateway, AddDeepSeekKey }
```
One concept, one representation: a missing CLI or a missing login is
`Unavailable::NotInstalled`/`NotLoggedIn` (it follows backoff and keeps the last reading as
`Stale`); `Setup` is only for a provider that has never been set up and has nothing to retry
(no gateway log yet, no DeepSeek key).
There is no `Known` without a reading, no `Stale` without the reading it refers to, no error
that carries a string. A provider cannot be "known and failing" at once; it is `Stale` with
`Failing(reason)`.

`Usage` is **derived**, never stored: `assess::derive(&SourceState, now, &FreshnessPolicy)`
is a pure function over what the service remembers **per provider** (the merge of its sources)
(`SourceState { last_good: Option<Reading>, last_failure: Option<(UnixMillis, SourceError)>, attempted: bool }`).
Rules: nothing attempted → `Pending`; a failure newer than `last_good` → `Stale{Failing}` when
there is a reading, else `Unavailable`/`NotConfigured`/`Unsupported` from the error; a reading
older than `fresh_for` with no newer failure → `Stale{Quiet}`. **Clock skew**: ages are
saturating differences, and the service clamps a reading's `observed_at` to `now` when it
arrives from the future (a gateway row stamped ahead of the wall clock, or a clock set
back), so a reading can never look fresh for longer than its policy and no sample is stored
with a future time.

### 4.4 Ports (traits core defines, io implements)
```rust
// port.rs
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum SourceId { ClaudeGatewayLog, ClaudeLive, CodexAppServer, DeepSeekBalance,
                    GeminiNone, AntigravityNone, GrokNone }
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Trigger {
    /// Fetch when the source signals that it has news (plus once at start and when the
    /// window becomes visible). The gateway log's file watcher signals; the later chat
    /// window's turn events signal the same way, with no change to this trait.
    OnSignal,
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
    /// Many readings in one transaction (the first-run backfill). Returns samples stored.
    fn record_all(&self, provider: ProviderKey, readings: &[Reading]) -> Result<u64, StoreError>;
    fn series(&self, provider: ProviderKey, meter: &MeterId, from: UnixSeconds, to: UnixSeconds,
              max_points: u16) -> Result<SeriesData, StoreError>;   // points, reset marks, bucket width
    fn load_alerts(&self) -> Result<AlertMemory, StoreError>;
    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError>;
    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError>;
}
pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Settings;                              // defaults when missing or invalid
    fn save(&self, s: &Settings) -> Result<(), StoreError>;
}
/// MUST NOT block: an implementation hands the alert to its own worker and returns.
pub trait Notifier: Send + Sync { fn notify(&self, alert: &Alert); }
```
`HistoryStore` and `SettingsStore` are synchronous and do file IO; the service calls every
method of both through `spawn_blocking`.
A provider may have several sources (Claude: gateway log and the opt-in live check). The
service keeps the reading with the greatest `observed_at`; a reading older than the one it
holds is dropped.

### 4.5 Policy (pure functions in core)
- `assess::derive(state, now, policy) -> Usage` and `assess::phase(window, now) -> Phase`
  (`Current` or `Elapsed` when `resets_at <= now`).
- `assess::severity(used, thresholds) -> Severity` (`Ok | Warn | Critical`), and `Unknown` for
  an elapsed or absent meter. A balance meter is `Critical` when `usable` is false, else `Ok`.
- `alerts::step(memory, provider, reading, now, settings) -> (AlertMemory, Vec<Alert>)`:
  section 7.6.
- `summary::headline(&[ (ProviderKey, Usage) ], now, thresholds) -> Option<Headline>`: the
  window meter with the highest `used` among `Known` and non-elapsed `Stale` readings of
  enabled providers.
- `history::should_store(last_stored, candidate) -> bool`, `history::downsample(points, max)`,
  `history::reset_marks(sorted_resets, kind) -> Vec<UnixSeconds>` (collapses a sliding
  `resets_at` into one mark per window instance: a new mark needs a jump of at least half the
  window length, or 1 hour for a window of unknown length; at most 64 marks),
  `history::gap_for(bucket) -> Duration` (`max(75 min, 2.5 x bucket)`: the longest hole the
  chart bridges; 75 min is 2.5 x the longest polling interval).
- `text::*`: the only place tray and notification strings are built (section 9.9).
- `settings::Settings::validated(raw) -> Settings`.

### 4.6 SOLID mapping
| principle | how it is met |
|---|---|
| Single responsibility | One module, one reason to change: each `parse::*` knows one wire format; `assess` knows freshness; `alerts` knows thresholds; `tail` knows files; `store` knows SQL; `tray` knows the tray. |
| Open/closed | New provider = new `UsageSource` + one registry line + one `ProviderKey` variant. The service, store, IPC and UI render from `Meter` generically. |
| Liskov | Every `UsageSource` returns the same `Reading`/`SourceError`; the service never matches on `SourceId` to behave differently. Test: the service test-suite runs against a fake source and every real adapter driven by fixtures. |
| Interface segregation | Four small ports in core (`UsageSource`, `HistoryStore`, `SettingsStore`, `Notifier`) plus `Clock`; three in io for its own IO seams (`WallTimer`, `EngineProbe`, `Jitter`). The UI gets one read model (`Snapshot`). |
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
exclude = [".claude", "dashboard/fuzz"]   # see below
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
- **`exclude` is required, not optional.** Agent worktrees of other branches live inside the
  repository at `.claude/worktrees/*`. Once the root manifest has a `[workspace]` table,
  cargo run inside a worktree whose own manifest has none walks up, finds this root, and
  fails with "current package believes it's in a workspace when it's not". Excluding
  `.claude` keeps every existing branch buildable. `dashboard/fuzz` is excluded because it
  needs nightly and its own lockfile.

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
  packaging/                       dev.jev.flashboard.desktop
  fuzz/                            cargo-fuzz targets (optional stage; excluded from the workspace)
  e2e/                             Playwright (mock IPC), the real-binary smoke test, and optional WebDriver tests
  scripts/                         install.sh, measure-idle.sh
  package.json, pnpm-lock.yaml, .npmrc, vite.config.ts, vitest.config.ts, playwright.config.ts, tsconfig.json
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

What these engine functions really do (read from the source, and the reason for the honest
limits in 8.3, 8.5 and 8.6):
- `state_dir()` **creates** `~/.local/state/jev-router` when it is missing. Harmless (the
  gateway would create the same directory) and, after `hardening::apply`, created 0700.
- `path_from_env` returns a `String` and looks the variable up in the process environment,
  then in the env file, which it reads whole into an ordinary `String`.
- `probe_all()` probes all five engine providers: it runs `claude auth status` and
  `codex login status` with the app's **own environment and working directory** (not the
  scrubbed ones of 8.6) and captures their stdout and stderr; it parses the JSON of
  `claude auth status` (which contains the account e-mail) and keeps `loggedIn` and the plan;
  it reads `~/.codex/auth.json` and decodes the `id_token` claims to get the plan name; it
  reads `~/.codex/models_cache.json` and `~/.gemini/settings.json`; it looks the provider keys
  up through the env file. `Availability` carries free strings (`detail`, `models`, `plan`).
  `EngineFacts::from_availability` keeps exactly two things per provider, the `State` and the
  plan as a `Label`, and drops the rest before anything else can see it.
- `claude` and `codex` are found through `PATH`. Under a mise command-wrapper for a tool that
  is listed but not installed, running the status command would start mise's installer; the
  probe kills it after 5 s. This is existing engine behaviour that the dashboard inherits,
  which is why the dashboard re-probes rarely (7.2).

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
  view ipc                  settings_file                    bus       (the one D-Bus user: tray host)
                            timer          (wall-clock timer) localtime (LocalTime impl)
                            registry       (source list)     cli       (--hidden)
                            service        (the one loop)
```

## 6. Sources (adapters)

| source | trigger | cost | reads | never touches |
|---|---|---|---|---|
| `ClaudeGatewayLogSource` | signal: file change on `usage.jsonl` | free | `ts_ms` and the allow-listed `ratelimit` keys | every other key; `gateway.jsonl`, `turns.jsonl`, `history`, `sessions/` |
| `ClaudeLiveSource` | manual, opt-in, off by default | **spends a tiny request** | `rate_limit_event` lines of one `claude -p` run | the answer text (discarded unparsed), `claude auth status` output |
| `CodexAppServerSource` | interval 5 min visible / 15 min hidden | free | `account/rateLimits/read` result | `account/read`; `~/.codex/*` (the engine's probe does read two files there, 5.1) |
| `DeepSeekBalanceSource` | interval 10 min visible / 30 min hidden | free | `GET /user/balance` body | anything else on the network |
| `PlaceholderSource` (Gemini, Antigravity, Grok) | never | free | nothing | everything (returns `Unsupported`) |

### 6.1 Claude gateway log
- Path from `embed::path_from_env("JEV_USAGE_LOG", "usage.jsonl")`, resolved once at start;
  previous generation at `<path>.1`. Opened read-only. The source never creates, truncates,
  locks or writes either file.
- **Row order**: rows are not in `ts_ms` order (3.2). "Newest" always means the greatest
  `ts_ms`. The source keeps, per window name, the latest value it has seen and the `ts_ms` it
  came from; a row only replaces a window's value when its `ts_ms` is greater.
- **Stable meter set**: a row does not carry every window (`7d_oi` is on a few rows only).
  The `Reading` the source returns has one meter per window it currently holds: the windows
  of the newest row, plus any window carried from an earlier row while that window's
  `resets_at` is still in the future and the row is younger than 7 days. So a window does not
  flicker in and out of the card. At most 8 meters (the 8 most recently updated).
  `observed_at` is the greatest `ts_ms` among the rows used.
- **Cold start**: read backwards from EOF in 256 KiB blocks, at most 16 MiB. Stop after the
  first block that contains at least one sample, and feed **every** sample row of the blocks
  read (not only the last line) to the per-window state above. If the file has none, repeat
  on `<path>.1`; if none, `NotConfigured(UseGateway)`.
- **Tail**: remember `(device, inode, offset)`. On a signal: `stat`; if the inode changed or
  the size shrank, finish the old file from the remembered offset through `<path>.1` when its
  inode matches, then start the new file at 0. Read forward in 256 KiB blocks, split on
  `\n`, keep the incomplete last line for next time. A line longer than 1 MiB is skipped to
  its newline without being buffered. Memory is bounded. When a read brings no newer sample
  (rows without `ratelimit`, or nothing new) the source returns the reading it already holds;
  it never turns a known reading into an error because a signal carried no sample.
- **Missing file or directory**: no file → `NotConfigured(UseGateway)`. The watcher (7.2)
  keeps trying, so the card fills in as soon as the gateway writes its first row.
- **Parsing** (`parse::claude_log::parse_line(&[u8]) -> Option<ClaudeLogSample>`): a serde
  struct with exactly two fields, `ts_ms: u64` and `ratelimit`, where `ratelimit` has a
  hand-written visitor that (a) ignores every key that does not start with
  `anthropic-ratelimit-unified-`, (b) converts each accepted value to a number or a closed
  enum immediately, (c) keeps no string except a window name that passes `Label::parse`.
  All other keys are skipped by serde without being materialised into owned values.
- **Key rule** (exact): strip the prefix, then split the rest at its **last** `-` into
  `(window, attribute)`. Only the attributes `utilization`, `reset` and `status` are read.
  A key with no `-` (`status`, `reset`) has no window and is ignored. So
  `5h-surpassed-threshold` (attribute `threshold`), `fallback-percentage` and
  `representative-claim` are ignored, and `overage-status` gives a window with a status but
  no utilization, which yields no meter. A window needs a parsable `utilization`; an
  unparsable `reset` leaves `resets_at: None`.
- Window names: `5h` → `FiveHour`, `7d` → `SevenDay`, anything else → `Named(label)` when it
  passes `Label::parse` (so `7d_oi` is shown as a named meter), otherwise dropped. The plan
  comes from the engine's availability probe (`max_5x`), not from the log.
- **First-run backfill** (once, recorded in the store's `meta`): stream `<path>.1` then
  `<path>` on a blocking thread and insert history samples in batches (`record_all`) under
  the normal write rule of 7.5, which drops rows older than the last stored one. It runs
  **before** the first live Claude sample is recorded (the service holds back `record` for
  that provider until it finishes; display is not held back), because a sample stored first
  would make every backfill row "older than the last stored". Never repeated. Bounded by the
  2 x 50 MB rotation.
- Freshness: `Known` while the newest sample is younger than 10 minutes; then
  `Stale{cause: Quiet}`; an individual window whose `resets_at` has passed is shown as
  `Elapsed` (section 9.5). This is honest: usage outside the gateway is invisible to it.
  A gateway that is not running looks exactly like a quiet one, which is correct.

### 6.2 Claude live check (opt-in)
Runs `claude -p` once with the cheapest model and a one-word prompt and reads only
`rate_limit_event` lines (`parse::claude_event`). Disabled by default; enabling it shows a
confirmation that it spends a small request each time. Only ever triggered by the user's
click. Exact flags are confirmed by the worker from `claude --help` only (cheapest model,
no tools, and no session file if the CLI has a flag for it). Per 1a it is built in v1, off by
default, and the lead may verify it **once** with one tiny Haiku request; no other real run
happens during development. Honest limit: the child gets no `ANTHROPIC_BASE_URL` from us
(8.6), but if the user's own Claude settings file routes Claude to the gateway, this request
goes there like any other Claude request; the dashboard never opens that file to check.

### 6.3 Codex app-server
Spawn `codex app-server` (section 8.6 rules), write the three request lines of
`dashboard/fixtures/codex-app-server/requests.jsonl` (without `account/usage/read` and
without `account/read`), read lines until the response with the `rateLimits/read` id, close
stdin, wait for exit (kill after the 15 s deadline). Line cap 1 MiB, total cap 4 MiB.
`parse::codex::parse_rate_limits_response(line, id)` (signature in T01) reads `rateLimits`
only: one window meter per non-null `primary`/`secondary`
(`WindowKind::from_minutes(windowDurationMins)`, absent duration →
`Named("primary")`/`Named("secondary")`; when both map to the same kind the second is
dropped), `usedPercent` as any JSON number (integer or fraction, 0 and up; negative or
non-finite drops the window), `resetsAt` null → `resets_at: None`, `plan` from `planType`
through `Label::parse`. Status: `Rejected` on **every** window when `rateLimitReachedType` is
non-null or `ordinaryUsageAllowed` is false, else `Allowed`. `credits.balance` (assumed to be
a decimal string; it was `null` here) becomes a `Balance(Usd)` meter only when `hasCredits`
is true, `unlimited` is false and the string parses. A response with no usable window and no
balance is `Protocol`. A JSON-RPC `error` response is `ServerError`; a malformed response is
`Protocol`.
Gate: spawn only when the engine's probe says Codex is `Ready`. Otherwise, without spawning
anything: `NotInstalled` → `Unavailable(NotInstalled)`, `NotLoggedIn` →
`Unavailable(NotLoggedIn)`, `Unknown` (the probe timed out or could not read the status) →
`Unavailable(Timeout)`, and the next attempt re-probes first (7.2).

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
shared mutable state, no other task mutates anything. **Everything it touches is injected**,
so every behaviour is testable with fakes:
```rust
pub struct ServiceDeps {
    pub clock: Arc<dyn Clock>, pub timer: Arc<dyn WallTimer>, pub jitter: Arc<dyn Jitter>,
    pub sources: Vec<Arc<dyn UsageSource>>,
    pub probe: Arc<dyn EngineProbe>, pub facts: Arc<FactsCell>,       // engine availability (io::engine_bridge)
    pub store: Arc<dyn HistoryStore>, pub settings: Arc<dyn SettingsStore>,
    pub notifier: Arc<dyn Notifier>, pub signals: mpsc::Receiver<SourceId>,
    pub backfill: Option<Backfill>,                                    // None when already done
}
/// The first-run history import of one provider; `run` executes on a blocking thread.
pub struct Backfill { pub provider: ProviderKey,
                      pub run: Box<dyn FnOnce() -> Result<u64, StoreError> + Send> }
pub struct ShellFacts { pub app_version: &'static str, pub tray_available: bool }
pub struct ServiceHandle { /* cloneable */ }
impl ServiceHandle {
    pub fn snapshot(&self) -> Snapshot;                               // latest, lock-free read (watch channel)
    pub fn subscribe(&self) -> watch::Receiver<Snapshot>;
    /// Queues the fetches and returns; never waits for a fetch to finish (7.2).
    pub async fn refresh(&self, target: RefreshTarget) -> Result<(), CommandError>;
    pub async fn history(&self, q: HistoryQuery) -> Result<HistorySeries, CommandError>;
    pub fn settings(&self) -> Settings;
    pub fn subscribe_settings(&self) -> watch::Receiver<Settings>;
    pub async fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, CommandError>;
    pub fn set_visible(&self, visible: bool);
    pub async fn diagnostics(&self, shell: ShellFacts) -> Diagnostics; // reads store statistics off the loop
    /// Cancels in-flight fetches (killing their process groups) and stops the task. Bounded: 2 s.
    pub async fn shutdown(&self);
}
pub fn spawn(deps: ServiceDeps) -> ServiceHandle;

// io::timer: the one place that waits for a wall-clock instant
pub enum Wake { Due, ClockChanged }
pub type TimerFuture<'a> = Pin<Box<dyn Future<Output = Wake> + Send + 'a>>;
pub trait WallTimer: Send + Sync { fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_>; }
pub struct SystemTimer;   // timerfd(CLOCK_REALTIME, TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET) in tokio's AsyncFd, via rustix
pub struct FakeTimer;     // feature "test-support": resolves when the FakeClock reaches the deadline
```
The loop is a single `select!` over: the command channel, the signal channel, completed
fetches, and **one** `timer.sleep_until(next_deadline)` where `next_deadline` is the minimum
of every interval source's next due time, the next window reset (to flip a meter to `Elapsed`
and recompute the headline), the next freshness boundary, and the daily prune. With nothing
due, the task is parked: zero wakeups.

**Why a wall-clock timer and not `tokio::time::sleep_until`.** Every deadline here is a
wall-clock instant (a reset time, "10 minutes after this sample"). Tokio's timer counts
`CLOCK_MONOTONIC`, which stands still while the machine is suspended: after an 8-hour sleep a
"reset at 14:00" timer would fire 8 hours late and the tray would show a window that ended
long ago. A `CLOCK_REALTIME` absolute timerfd fires as soon as the machine resumes past the
deadline, and `TFD_TIMER_CANCEL_ON_SET` wakes the loop (`Wake::ClockChanged`) when the wall
clock is set (NTP step, manual change), so it recomputes every deadline from `clock.now()`.
Fetch timeouts stay on tokio's monotonic timer, which is the right clock for a duration.

### 7.2 Refresh strategy
- **Event-driven first**: `watch` uses inotify (`notify` crate) on the log's parent directory
  (so rotation by rename, deletion and re-creation are seen), filtered to the log's file
  name, and coalesces a burst into at most one signal per 1.5 s (trailing edge). No polling
  of the log, ever. An inotify queue overflow counts as a change. If the directory does not
  exist yet, or the watch is lost (directory removed), the watcher re-arms itself at
  1, 2, 4 … 15 minute intervals until it succeeds, signals once when it does, and emits
  `DiagEvent::WatchLost`; no fetch loop is involved.
- **Signal sources** are also fetched once at start and once whenever the window becomes
  visible (a `stat`, in case a signal was lost).
- **Intervals** only for sources with no push channel (Codex, DeepSeek), longer while the
  window is hidden. A refresh also runs when the window becomes visible if the reading is
  older than the visible interval.
- **Backoff**: on failure the next attempt is `min(60 min, 1 min x 2^failures)` with ±10 %
  jitter from the injected `Jitter`; success resets it. `RateLimited` and `Unauthorized`
  start at 15 min. `NotConfigured` and `Unsupported` are not failures: they are re-checked at
  the source's normal interval and never count towards backoff.
- **Manual refresh** (`refresh` command, tray "Refresh now"): `all` starts every enabled
  source that costs nothing, is not in flight and whose last attempt ended more than 10 s
  ago; it always returns `Ok`. `provider` does the same for one provider and returns `Busy`
  when all of its free sources are in flight. `live_check` starts the spending source of
  that provider: `NotEnabled` when the setting is off or the provider has none, `Busy` when
  it is in flight. A manual refresh bypasses backoff. The call returns as soon as the
  fetches are queued; progress is `RefreshView.in_flight` in the snapshot.
- **One fetch per source at a time**, each under its descriptor's timeout. Dropping a fetch
  (timeout, shutdown, provider hidden) kills its child process group (8.6).
- **Engine probe** (`EngineProbe`, which runs `claude auth status` and `codex login status`):
  (a) once at start; (b) before a gated fetch when the cached readiness of that provider is
  not `Ready` and the cached facts are older than 60 s, so a probe that timed out during a
  busy login is retried on the source's backoff schedule **even while the window is
  hidden**; (c) on a manual refresh, at most once per 60 s; (d) otherwise when the facts are
  older than 6 hours, piggy-backed on a fetch that is due anyway (no timer of its own). The
  probe never runs more often than once per 60 s.
- **Suspend/resume and clock jumps**: handled by the wall-clock timer (7.1). Every wake
  recomputes deadlines from the injected clock; deadlines found in the past run once each
  (no catch-up storm).
- The UI's countdown text is recomputed in the web view only while the window is visible
  (one timer aligned to the minute, stopped on `visibilitychange`). The backend never pushes
  ticks. Tray text uses absolute times, so it needs no ticking either.

### 7.3 Tauri command surface (complete)
All in `jev-dashboard::commands`. Arguments and results are the generated types of section
7.4. Errors are the closed enum `CommandError`; no command returns a string.

| command | args | result | notes |
|---|---|---|---|
| `get_snapshot` | none | `Snapshot` | synchronous read of the latest state |
| `refresh` | `target: RefreshTarget` | `Result<(), CommandError>` | returns when queued (7.2); the result arrives as a `snapshot` event |
| `get_history` | `query: HistoryQuery` | `Result<HistorySeries, CommandError>` | at most 600 points |
| `get_settings` | none | `Settings` | |
| `update_settings` | `patch: SettingsPatch` | `Result<Settings, CommandError>` | validated in core; the shell then applies the autostart entry (9.13) |
| `get_diagnostics` | none | `Diagnostics` | async; version, per-source state, store size |

Events (Rust → UI only): `snapshot` (payload `Snapshot`, emitted when it changes) and
`navigate` (payload `Route`, from the tray menu). The UI emits no events. There is no
command to quit, open a URL, read a file, run a process, or read the clipboard.

**Start-up order in the UI** (no lost update): subscribe to `snapshot` first, then call
`get_snapshot`, and keep whichever has the greater `generated_at`.
**Route delivery** (no lost navigation): an event sent to a web view that is still loading is
lost, so when the window has to be created the shell passes the route in the URL fragment,
a `&`-separated list of `key=value` pairs from a closed vocabulary: `view`
(`overview` | `settings` | `provider`), `provider` (a `ProviderKey`) and `theme` (a `Theme`),
for example `index.html#view=provider&provider=codex&theme=dark`. The UI reads it once at
boot and ignores anything else; `navigate` is used only for a window that already exists.

### 7.4 IPC types: exactly what crosses the boundary
Defined in `jev-usage-core::ipc`, `#[serde(rename_all = "snake_case")]`, tagged enums with
`#[serde(tag = "state")]`/`"kind"`. TypeScript is generated by `ts-rs` into
`dashboard/src/lib/ipc/bindings/` and checked for drift in CI. `u64`/`i64` fields carry
`#[ts(type = "number")]` (all values are far below 2^53). The block below shows the shapes;
where `ts-rs` prints an equivalent type differently, the generated files are authoritative.
`SettingsPatch` fields carry `#[serde(default)]` and `#[ts(optional)]`, so an absent key
means "unchanged".

```ts
type ProviderKey = "claude" | "codex" | "deepseek" | "gemini" | "antigravity" | "grok";
type Severity    = "ok" | "warn" | "critical" | "unknown";
type Phase       = "current" | "elapsed";
type LimitStatus = "allowed" | "warning" | "rejected" | "unknown";
type Unavailable = "not_installed" | "not_logged_in" | "offline" | "timeout" | "rate_limited"
                 | "unauthorized" | "server_error" | "protocol" | "io" | "internal";
type Setup       = "use_gateway" | "add_deepseek_key";
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
                       unit: "basis_points" | "micros"; points: HistoryPoint[];
                       gap_s: number;       // a hole longer than this is drawn as a gap (history::gap_for)
                       resets: number[] };  // window ends inside the range (epoch s), at most 64; empty for month and quarter

type Theme     = "system" | "light" | "dark";
type HourCycle = "auto" | "h12" | "h24";
type Settings  = { version: number /* 1 */; warn_bp: number; critical_bp: number;
                   notifications: boolean;              // master switch (also the tray checkbox)
                   notify_warn: boolean; notify_critical: boolean;
                   theme: Theme; hour_cycle: HourCycle;
                   close_to_tray: boolean; autostart: boolean; shortcuts: boolean;
                   claude_live_check: boolean; first_run_seen: boolean;
                   hidden_providers: ProviderKey[] };
type SettingsPatch = Partial<Omit<Settings, "version">>;
type CommandError  = { code: "invalid_argument" | "busy" | "not_enabled" | "store_unavailable" | "io" | "internal" };
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
(section 8.10). `Diagnostics.sources[].last_ok_at == null` is how an adapter that has never
succeeded (DeepSeek before a key exists) shows as unverified. The window size is **not** in
`Settings`: it is shell state kept in the store's `meta` table (7.5).

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

- File: `<state_dir>/dashboard/history.db`. The directory is created 0700 and tightened if
  looser; `history.db`, `-wal` and `-shm` are 0600. SQLite gives the WAL and SHM files the
  mode of the main database file, so the store creates the main file 0600 itself (open with
  mode 0600 before SQLite touches it) and does not rely on the process umask; a test asserts
  all three modes with a umask of 022. `journal_mode=WAL`, `synchronous=NORMAL`,
  `auto_vacuum=INCREMENTAL`, `foreign_keys=ON`, `secure_delete=ON`, `busy_timeout=2000`.
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
- Write rule (`history::should_store`): a candidate **not newer** than the last stored
  sample of that meter (same or earlier second) is dropped, which makes out-of-order log
  rows and repeated inserts harmless. Otherwise store when the value or `resets_at` differs
  from the last stored sample, or when 15 minutes have passed since it (a heartbeat, so the
  chart can tell "flat" from "no data").
- `series`: aggregation to at most `max_points` buckets is done **in SQL** (`GROUP BY` bucket
  with min, max and last), so a query never loads the raw rows of a range into memory. It
  returns the points, the bucket width, and the reset marks: the distinct `resets_at_s` of
  raw samples inside the range, collapsed by `history::reset_marks`. Marks come from raw
  samples only, so the month and quarter ranges return none.
- `meta` also holds the shell's last window size (`MetaKey::WindowWidth`,
  `MetaKey::WindowHeight`): two integers, no new file and no new IPC field.
- Retention: raw samples 45 days (covers the 30-day Codex window); hourly rollups
  (`lo/hi/last`) 400 days; prune at start and every 24 h in one transaction, then
  `PRAGMA incremental_vacuum`. Expected size: well under 5 MB.
- Migration: `PRAGMA user_version`; an ordered list of `(version, sql)` applied in one
  transaction; a database newer than the binary is opened read-only for charts and not
  written; a corrupt database is renamed to `history.db.corrupt-<epoch>` and recreated
  (history is derived data; losing it never breaks the app).
- Access: one connection behind a mutex, every call through `spawn_blocking`. A second
  instance cannot open it: `tauri-plugin-single-instance` ends a second process before the
  store is opened (8.8).

### 7.6 Alerts (pure state machine)
For each window meter and each enabled threshold (warn, critical):
- **Fire** when `used >= threshold`, the reading is fresh (observed within 10 minutes), the
  window is not elapsed, and `AlertMemory` has no entry for `(meter, threshold)`.
- If both thresholds are crossed by the same reading, fire **only critical** and record both.
- A crossing is **recorded even when its notification is switched off** (master switch or
  that level), so turning notifications on later does not replay old crossings. Only the
  `Alert` is suppressed.
- No alerts for a hidden provider or for a balance meter.
- **Re-arm** (delete the entry) when `used < threshold - 5 points`, or when the recorded
  `window_resets_at` has passed. This is robust whether or not a provider's `resetsAt` is
  stable between reads.
- `AlertMemory` is persisted, so a restart never repeats a notification.
- Nothing fires during the first-run backfill or for `Stale`/`Elapsed` data.
Result: at most one notification per threshold per window instance.
The UI's live region announces the same level changes when the window is open, derived from
the `severity` of consecutive snapshots and worded like the notification (9.10), so a
screen-reader user hears what a sighted user sees change, once. No extra IPC is needed.

### 7.7 Caching, errors, staleness
- The latest `Snapshot` lives in a `watch` channel: `get_snapshot` is a clone, never IO.
- A snapshot event is emitted only when the snapshot differs from the previous one,
  ignoring `generated_at`. Refresh bookkeeping (`in_flight`, `next_at`) changes on every
  poll, so the **tray** does not rebuild on every snapshot: it compares its own derived
  value (menu row strings plus `IconSpec`) and touches the menu or icon only when that
  changes.
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
- Tests use `FakeClock` (settable, advanceable) with `FakeTimer`, and tokio's paused time
  for fetch timeouts; no test sleeps.
- Timezone or DST change: nothing stored or scheduled depends on the zone. The UI re-renders
  local times on its minute tick; tray rows are rebuilt at the next content change, and their
  format (9.11) has no "today"/"tomorrow" to go wrong in between.

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
| The web view is compromised anyway | It can call six commands that return numbers. No shell, fs, http, opener, clipboard, dialog, notification or updater plugin is compiled in. Capabilities grant event-listen only. It runs with an ephemeral (incognito) data store. |
| A dependency is malicious or vulnerable | Section 8.8. |
| Another local process of the same user reads our files or memory | Out of scope in general (same-uid processes can read `~/.jev-router.env` directly). We still write 0600/0700, disable core dumps and ptrace-attach (`PR_SET_DUMPABLE 0`), and keep secrets out of files. |
| The app spends quota or disturbs the gateway | Read-only on gateway files; no request through the gateway; the single spending path (`ClaudeLive`) is opt-in, manual, labelled. |
| A lazy installer shim runs (mise wrappers for `grok`, `agy`) | 8.6. |
| Secrets inherited by a child process | Children the dashboard spawns get a scrubbed, allow-listed environment (8.6). Honest limit: the engine's probe runs `claude auth status` and `codex login status` with the app's own environment (5.1); these are the user's own two CLIs. |
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
  opened by dashboard code (architecture test). **Honest limit, from the engine source
  (5.1):** the engine's probe, which the dashboard calls, does read `~/.codex/auth.json` and
  decodes the `id_token` claims in memory to find the plan name (it does not verify, store
  or forward the token), reads `~/.codex/models_cache.json` and `~/.gemini/settings.json`,
  and checks that `~/.gemini/oauth_creds.json` and `~/.grok` exist.
- `claude auth status` output (JSON that includes the account e-mail) is read only by the
  engine's existing parser, which keeps the logged-in flag and the plan and drops the rest.
  The e-mail therefore passes through engine heap memory for a moment (ordinary, not
  zeroized buffers). It never reaches a dashboard type: `EngineFacts` holds a `State` and a
  `Label` per provider and nothing else, so it cannot be displayed, stored, logged or sent
  over IPC, which is what 1a requires.
- Codex `account/read` is never called.
- From `~/.jev-router.env`: the dashboard's own reader keeps only the `DEEPSEEK_API_KEY`
  line's value; the file text lives in a zeroizing buffer for the duration of the parse.
  Honest limit: the engine functions the dashboard calls (`path_from_env`, `probe_all`) read
  the same file into ordinary strings to look up their own settings and to see whether keys
  exist (5.1); those copies are dropped, not zeroized.

### 8.4 What is never stored, logged or sent
- **Stored** (history.db, settings.json): numbers, booleans, closed-vocabulary keys. Nothing
  else. No paths, no timestamps finer than a second for samples. Two other things reach the
  disk and are stated here so the list is complete: (1) the autostart entry
  `~/.config/autostart/dev.jev.flashboard.desktop`, which has to contain the binary's
  absolute path (so the home directory name), in a file only the user reads; (2) tray icon
  images, which the tray library writes as PNG files under `$XDG_RUNTIME_DIR` (a ring gauge,
  no text). The web view stores nothing: it is created with an ephemeral (incognito) data
  store, and the UI is forbidden by lint from using `localStorage`, `sessionStorage`,
  `indexedDB`, cookies or the Cache API.
- **Logged**: the app has one diagnostic channel, `diag::emit(DiagEvent)`, where `DiagEvent`
  is an enum whose fields are numbers and closed enums. There is no API to log a string.
  `println!`, `eprintln!`, `dbg!`, `log`/`tracing` macros are denied by lint in the three
  crates (`clippy::print_stdout`, `print_stderr`, `dbg_macro`, `disallowed_macros`).
  Output goes to stderr only; the app writes no log file. Honest limits: when the app is
  started by the session (autostart, the launcher) the session's journal keeps stderr, so
  these lines (numbers and enum names) do end up on disk there; and GTK and WebKit print
  their own warnings to the same stderr, which the app does not control.
- **Errors**: every error type is a field-less or numeric enum. No `anyhow`, no
  `Box<dyn Error>`, no `to_string()` of a foreign error crosses a module boundary
  (foreign errors are classified at the adapter and dropped).
- **Panics**: `panic = "abort"` in `dist`; the panic hook prints only the source file's
  **base name** and the line, never the payload and never the directory (a dependency's
  location is an absolute path under `~/.cargo`, which contains the user name). No crash
  reporter, no upload. Core dumps disabled.
- **IPC**: section 7.4 is exhaustive.
- **Notifications and tray**: provider name, window name, percentage, reset time. Built only
  by `core::text` from typed inputs.
- **Clipboard**: written only by the Copy buttons, only with constant command strings from
  `copy.ts` (`codex login`, `jev-claude`, `DEEPSEEK_API_KEY=`); never read.
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
    /// The single way out. Call sites are counted by a test: exactly one, in `io::http`,
    /// where the Authorization header is built.
    pub fn expose_secret(&self) -> &T;
}
impl<T: Zeroize> Drop for Secret<T> { /* zeroize */ }
```
- The key is read at fetch time, used for one request, and dropped (zeroized) when the fetch
  ends. It is not cached between fetches and never placed in a struct that outlives the call.
- It is read by `io::secrets::deepseek_key(&KeySources) -> Option<Secret<String>>` (the
  sources are injected, so tests never touch the real environment or home): process environment
  first, else the env file read into a `Zeroizing<String>` and parsed with the engine's pure
  `embed::parse_env` (DRY: same parsing rules as the engine).
- The header value is marked sensitive (`HeaderValue::set_sensitive(true)`), so the HTTP
  stack will not print it.
- Honest limits: the allocator may have left copies in freed memory before zeroization of
  intermediate buffers we do not own (the engine's parser returns a `String` we move, not
  copy; `std::env::var` returns a copy we own and zeroize). The engine's own probe looks the
  same key up to decide availability and drops it without zeroizing (5.1). The TLS stack
  holds the header bytes until the request is written. Swap is outside our control.

### 8.6 Subprocesses
One module, `io::proc`, is the only place `std::process`/`tokio::process` may be used
(architecture test). Rules:
- Program names come from a closed enum (`Program::Codex`, `Program::Claude`). There is no
  variant for `grok`, `agy`, `gemini` or `mise`, so they cannot be run.
- A program is spawned only if the engine's probe reports its provider `Ready` (the probe
  decides from files and from `claude auth status`/`codex login status`, and never runs
  `grok`, `agy` or `gemini`).
- Arguments are constant arrays. No shell, no string interpolation.
- Environment: `env_clear()` then exactly these, each only if set in the parent: `HOME`,
  `USER`, `LOGNAME`, `PATH`, `LANG`, `LC_ALL`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
  `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS` (a CLI
  that keeps its login in the session keyring needs the bus), `CODEX_HOME`,
  `CLAUDE_CONFIG_DIR`, `MISE_DATA_DIR`, `MISE_CONFIG_DIR` (so the child finds the same login
  and the same installed tool the engine's unscrubbed probe saw; all four are directory
  paths, not secrets); plus the constant `TERM=dumb`. No `*_API_KEY`, `*_TOKEN` or proxy
  variable ever reaches a child. For `ClaudeLive`, `ANTHROPIC_BASE_URL` is not set, so it
  does not go through the gateway.
- Scope: these rules bind every process the **dashboard** spawns. The engine's probe spawns
  its two status commands itself, with the app's environment (5.1); that is disclosed, not
  hidden, and the child-environment test covers `io::proc` only.
- Working directory: `<state_dir>/dashboard/run` (empty, 0700), so no project config is read.
- stdin/stdout piped, **stderr discarded**. Output caps (1 MiB per line, 4 MiB total).
  The child is the leader of its own process group. The group is killed (`SIGKILL`) on the
  deadline **and whenever the run is dropped** (service timeout, shutdown), by a guard that
  is disarmed once the child has been reaped; `kill_on_drop` alone would leave the real tool
  running behind its launcher (`codex` and `claude` are wrappers here).
- Child output is parsed by an allow-list parser and never logged or stored raw.

### 8.7 Network
- One module, `io::http`, is the only place `reqwest` may be used (architecture test). It
  exposes `get_json(endpoint: Endpoint, bearer: &Secret<String>) -> Result<Vec<u8>, Unavailable>`
  where `Endpoint` is a closed enum (`DeepSeekBalance`) mapped to a `const` URL. This is
  also the only place `expose_secret` is called.
- Client: rustls, `https_only(true)`, `redirect(Policy::none())`, `no_proxy()`, connect
  timeout 5 s, total timeout 10 s, body cap 64 KiB, no cookies, a fixed generic `User-Agent`
  (`jev-dashboard`), no other headers.
- Egress inventory (complete): (1) `api.deepseek.com` by this client, only when a key
  exists; (2) the `codex` child talks to OpenAI with its own login; (3) the opt-in `claude`
  child talks to Anthropic with its own login. Nothing else: no updater, no telemetry, no
  fonts or assets fetched at runtime, no link opening.

### 8.8 Tauri hardening
- Plugins compiled in: `tauri-plugin-single-instance` only, registered first. Because the
  service is created in Tauri's `setup` hook, which runs after plugin initialisation, a
  second process exits before it opens the store, the log or a child. **Not** compiled in:
  shell, opener, fs, http, dialog, clipboard, notification, updater, process, os,
  global-shortcut, window-state, autostart.
- Notifications are sent from Rust with `notify-rust` (the library the Tauri plugin wraps on
  Linux) and not through `tauri-plugin-notification`: the plugin's desktop API has no urgency
  hint, which 9.12 needs, and leaving it out removes its JavaScript command surface
  altogether. The worker confirms this against the pinned versions; if the plugin has gained
  an urgency setting, the lead may switch back.
- D-Bus is used directly in exactly one module, `shell::bus` (`zbus`, blocking calls on a
  blocking thread): asking whether a StatusNotifier host is registered (9.11). Nothing else.
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
- `tauri.conf.json`: `app.windows: []` (the `main` window is created in code, so `--hidden`
  creates no web view at all); `app.withGlobalTauri: false`;
  `app.security.freezePrototype: true`;
  `app.security.dangerousDisableAssetCspModification: false`; no `assetProtocol`; no remote
  URLs in `capabilities`; CSP:
  ```
  default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self';
  font-src 'self'; connect-src ipc: http://ipc.localhost; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'; object-src 'none'
  ```
  No `'unsafe-inline'`, no `'unsafe-eval'`, no `data:`. Consequences for the UI, all
  enforced by lint or test: Vite's `build.assetsInlineLimit` is `0` (no asset is ever inlined
  as a `data:` URI, which this CSP would block); icons are inline SVG in the markup; dynamic
  styles are set through Svelte's `style:` directive (the CSSOM), never through a `style="…"`
  attribute or an injected `<style>`, both of which this CSP blocks; animations are CSS.
- The window is built with `incognito(true)`: WebKit keeps cookies, storage and caches in
  memory only and leaves nothing under `~/.local/share` or `~/.cache`.
- Navigation is locked: `on_navigation` allows only the app origin; new windows are denied.
- Devtools: the `devtools` Cargo feature is not enabled; a test asserts the release manifest.
- The web view's context menu (which offers Reload and Inspect) is suppressed by the UI in
  production builds (a `contextmenu` listener in `main.ts`; Tauri has no Rust switch for it
  on Linux). A reload is harmless anyway: the UI rebuilds itself from `get_snapshot`.
- Exit: `RunEvent::ExitRequested` is prevented only when it has no exit code (the last window
  closed while the tray is alive); an explicit `app.exit(0)` from the tray's Quit goes
  through, after `ServiceHandle::shutdown()`.
- At start (`hardening::apply`), before any thread or child exists: `umask(0o077)`, core
  dumps off (`RLIMIT_CORE = 0`), `PR_SET_DUMPABLE=0`, the panic hook. Uses safe wrappers from
  `rustix`; `unsafe` is forbidden in core and io and denied in the shell. Children inherit
  the umask (their files become private, which is harmless). `PR_SET_DUMPABLE=0` makes
  `/proc/<pid>` owned by root; if the T11 manual check finds that this breaks a portal
  lookup or WebKit's sandbox helper on this machine, it is dropped and `RLIMIT_CORE = 0`
  stays (risk in section 14).

### 8.9 Dependency and supply-chain policy
- Rust: lockfile committed, every command `--locked`. `deny.toml`: advisories = deny
  (vulnerabilities, unsound, yanked), sources = crates.io only (no git), licenses = explicit
  allow-list, bans = `openssl`, `openssl-sys`, `native-tls` denied, wildcard versions denied.
  `cargo audit` as a second advisory source.
- Direct Rust dependencies, complete list (additions need lead approval):
  core: `serde`, `serde_json`, `zeroize`, `ts-rs` (dev/bindings feature); dev: `proptest`,
  `static_assertions`. io: `tokio`, `notify`, `rusqlite` (bundled), `reqwest` (rustls),
  `rustix`, `zeroize`, `serde_json`, `jev-router`; dev: `tempfile`, `proptest`. shell:
  `tauri` (feature `tray-icon` added to the defaults; dev: feature `test`), `tauri-build`,
  `tauri-plugin-single-instance`, `notify-rust` (default features off, `z` only: the zbus
  backend, no image support), `zbus` (the version `notify-rust` already resolves, so no
  second copy), `chrono` (no default features, `clock`), `rustix`.
- JavaScript: `pnpm-lock.yaml` committed, `--frozen-lockfile`; exact versions
  (`save-exact=true`); `.npmrc`/workspace settings: dependency build scripts blocked except an
  explicit allow-list (`esbuild`), `minimumReleaseAge` of 7 days for new versions;
  `pnpm audit --prod` must be clean. Runtime dependencies: `svelte`, `@tauri-apps/api`, two
  font packages (assets only). Everything else is a dev dependency (Vite, `svelte-check`,
  Vitest, Testing Library, `axe-core`, `@axe-core/playwright`, `@playwright/test`, ESLint,
  stylelint, Prettier, `@tauri-apps/cli`).
- No CDN, no remote font, no remote script: everything is bundled and served from the app.
- Build tools fetched at install time (`@tauri-apps/cli`) are pinned in the lockfile.

### 8.10 Tests that prove the rules
| rule | proof |
|---|---|
| No prompt can surface from the log parser | **Canary fixture**: `dashboard/fixtures/claude-gateway/canary.jsonl` has the canary string `JEVCANARY_9f3c1b7e_DO_NOT_SURFACE` in every non-allow-listed key, as extra keys, inside allow-listed keys' values, and as a window name. Test: `format!("{:?}")` and the JSON of everything `parse_line` returns contain no `JEVCANARY`. Property test: for arbitrary JSON rows with the canary at arbitrary depth, the same holds. |
| No secret or identity crosses any adapter | Every adapter test feeds canary-laced input (Codex `account_read.response.json` with `canary-email@example.invalid`, a DeepSeek response with the canary as an extra field, the canary as the DeepSeek key) and asserts the canary is absent from the returned `Reading`/error Debug output. |
| End to end: nothing leaks to IPC, store, settings, logs | `io` integration test: a service wired with canary-fed sources and a temp state dir runs a full cycle; then every emitted `Snapshot` (JSON), every file under the temp dir (raw bytes, including SQLite WAL), and captured `diag` output are searched for `JEVCANARY`, the canary e-mail and the canary key. The real-binary smoke test (11.4) repeats the search over its whole temp tree, including `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_CONFIG_HOME` and `XDG_RUNTIME_DIR`, and over the binary's stderr. |
| IPC carries only allowed fields | Test serialises generated values (proptest strategies for every IPC type) and walks the JSON: every key must be in `ipc-allowlist.txt`; no key matches `(?i)email|token|secret|password|prompt|message|content|account|session|cwd|path|url|auth|key$`; every string leaf must match a closed-vocabulary set or `^[a-z0-9_:.+-]{1,48}$`. The same walk runs over the generated `.ts` bindings' property names. |
| `Secret` cannot be printed/cloned/serialised | `static_assertions::assert_not_impl_any!(Secret<String>: Debug, Display, Clone, Serialize, PartialEq)`. |
| One exit for secrets, one HTTP client, one process spawner, one D-Bus user | Architecture test (plain Rust test that scans `dashboard/**/*.rs`, tests excluded where noted): `expose_secret(` only in `io/src/http.rs`; `reqwest` only in `io/src/http.rs`; `process::Command` only in `io/src/proc.rs`; `zbus` only in `src-tauri/src/bus.rs`; `notify_rust` only in `src-tauri/src/notifier.rs`; `jev_router::` only as `jev_router::embed`; forbidden file names (8.3) nowhere; no `println!`/`eprintln!`/`dbg!`; no `String`, `&str`, `Box<str>` or `Cow` in the core type files (`time quantity label provider meter usage port secret ipc`) except `label.rs`; `text.rs` builds display strings from typed inputs only and is not a type file. |
| Children the dashboard spawns get no secrets | Test spawns a stub program through `proc` with `DEEPSEEK_API_KEY=JEVCANARY...` in the parent environment; the stub prints its environment; assert no canary and only allow-listed names. (The engine probe's two status commands are outside this rule, 8.6.) |
| Files are private | Test asserts 0700 on the directory and 0600 on `history.db`, `-wal`, `-shm`, `settings.json`, with a process umask of 022. |
| The web view stores nothing | ESLint bans `localStorage`, `sessionStorage`, `indexedDB`, `document.cookie`, `caches`; the smoke test finds no WebKit data directory in its temp tree. |
| The panic hook leaks no path | Test in a child process: a panic with a canary payload prints a base name and a line, no `/`, no canary. |
| Capabilities and CSP do not drift | Snapshot tests of `capabilities/main.json`, the CSP string, the plugin list in `Cargo.toml`, and "no `devtools` feature". |
| The web view makes no network request | Playwright test fails on any request that is not the app origin or IPC; CSP violation events fail the test. The Playwright harness serves the built UI with the exact CSP string of `tauri.conf.json` as a response header (Tauri injects the CSP only at run time), so a CSP-breaking change fails here and not first on the user's machine. |
| No raw HTML sinks | ESLint rule bans `{@html}`, `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `eval`, `new Function`. |

## 9. UI/UX specification

### 9.1 Design intent
The user's bar (1a): **minimalistic but detailed and beautiful, also accessible**. Each word
is a rule the reviewers check.

- **Minimal.** An instrument, not a marketing page. Little chrome: no sidebar, no toolbar
  rows, no shadows on cards, no decorative icons or illustrations, one header line. Few
  colours: neutrals and **one** accent; amber and vermilion appear only when a level is
  reached, so a screen with nothing wrong is almost monochrome. Generous whitespace (card
  padding 16 to 24 px, 12 px between meter rows, 24 px between cards). One hierarchy per
  card, in this order and nothing else at rest: the number, the bar, one quiet line.
- **Detailed, on demand** (progressive disclosure, three levels):
  - *Level 0, always visible*: provider, plan, and per window the percentage, the bar and the
    time until reset; the freshness of the data.
  - *Level 1, on hover **or** keyboard focus of a meter row* (the same content either way):
    the exact figure to one decimal, the absolute reset time, the window's status, when the
    value was observed.
  - *Level 2, the detail view* (Enter or click on the card): history, every window in full,
    the source, the live check; Settings holds diagnostics.
  Nothing at level 1 or 2 is pointer-only or sight-only. A level-1 disclosure opens on focus
  as well as hover, stays open while the pointer is over it, closes on `Esc` without moving
  focus (WCAG 1.4.13), and never covers the control that opened it. Its complete content is
  **always** in the row's accessible text (`aria-valuetext`), so a screen-reader user gets
  the detail without opening anything, and a keyboard user reaches it with Tab alone.
- **Beautiful.** Considered typography (two families, a seven-step scale, tabular numerals
  that line up across cards), one 4 px spacing scale, concentric radii, optical alignment,
  hairlines instead of boxes, polished light **and** dark themes designed as equals, and
  motion that is subtle, short, tied to a value changing, and absent under reduced motion.
- **Accessible.** A hard gate, not a goal: WCAG 2.2 AA (9.10). A task that fails any item of
  9.10 is not done, however good it looks.

Workers implement this spec with the `frontend-design` and `impeccable` skills and finish
with a `better-ui` pass; deviations from tokens or wireframes need lead approval.

### 9.2 Information architecture
```
Overview (default)  ─┬─ Provider detail (one per provider)
                     └─ Settings ── Thresholds / Notifications / Appearance / Behaviour / Providers / About and diagnostics
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
│ │                   resets in 2h 14m                       │ │
│ │ 7-day             37%   █████████░░░░░░░░░░░░░░░░░       │ │
│ │                   resets in 2d 13h                       │ │
│ │                                         as of 12:33      │ │  freshness
│ └──────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ● Codex                                         Free   › │ │
│ │ 30-day             4%   █░░░░░░░░░░░░░░░░░░░░░░░░░       │ │
│ │                   resets in 29d 11h                      │ │
│ └──────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ○ DeepSeek                                               │ │
│ │ Add a key to see your balance.                           │ │
│ │ DEEPSEEK_API_KEY=…  in ~/.jev-router.env        [Copy]   │ │
│ └──────────────────────────────────────────────────────────┘ │
│  No usage interface yet: Gemini · Antigravity · Grok     ⓘ   │
└──────────────────────────────────────────────────────────────┘
```
- The wireframe shows level 0 (9.1). Hovering or focusing a meter row shows level 1 next to
  it: "50.0 % used · resets today 14:00 · status allowed · seen 12:33".
- The percentage is the largest element in a row (type `num-lg`), right-aligned in a fixed
  column so digits line up across cards (tabular numerals).
- The bar is 8 px high, full radius, with tick marks at the warn and critical thresholds.
- Layout is fluid (a tiling compositor decides the size): one column below 640 px, two
  columns from 640 px, three from 1100 px; card max width 520 px; page padding 16 px (24 px
  from 640 px). Minimum supported window 320 x 400.
- Each card is a group with a heading. Its header (name, plan, chevron) is one button that
  opens the detail view; a click anywhere on the card does the same for pointer users. Each
  meter row is its own tab stop (that is how the keyboard reaches level 1), and inner
  controls (Copy, Retry) are separate tab stops too. Interactive elements are never nested
  inside one another.
- The plan is shown by its display name ("Max 5x", "Free"): `copy.ts` maps the known plan
  labels and falls back to the label with `_` as spaces and the first letter capitalised.
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
                   │          resets in 12m                 │  a labelled note with a clock icon
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

FIRST RUN          centred, once:   Usage, in one place.
                   Claude and Codex appear automatically when they're set up.
                   [ ✓ Claude · found ]  [ ✓ Codex · found ]  [ ○ DeepSeek · add a key ]

                   Flashboard starts with your session and waits in the tray.
                   [x] Start at login                          (a real switch, applied at once)

                                                      [ Continue ]
```
- There is **no global "offline" banner**. A provider that cannot be reached says so on its
  own card (STALE failing), which is always true; a banner would need every network source
  to report "offline", and the Codex child cannot tell us that (its failures are a timeout
  or an error response).
- **First run** is shown while `settings.first_run_seen` is false and stays until the user
  presses Continue (it does not vanish on its own: the gateway log answers within
  milliseconds, and the autostart notice must be readable). Start at login is on by default
  (1a) and the entry has already been written when this is shown; the switch here turns it
  off again in one action, and the same switch lives in Settings. Continue sets
  `first_run_seen` and moves focus to the overview heading.
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
│   [x] Notify me when a window passes a level                 │
│       [x] Warn level      [x] Critical level                 │
│   Sent once per window. Never repeated.                      │
│ Appearance                                                   │
│   Theme   (•) System  ( ) Light  ( ) Dark                    │
│   Clock   (•) Auto    ( ) 12-hour ( ) 24-hour                │
│ Behaviour                                                    │
│   [x] Keep running in the tray when the window closes        │
│   [x] Start at login, hidden in the tray                     │
│   [x] Single-key shortcuts (R, comma, 1 to 6, ?)             │
│ Providers                                                    │
│   [x] Claude   [x] Codex   [x] DeepSeek                      │
│   [ ] Claude live check  · uses one small request per check  │
│ About and diagnostics                                  ▸     │
│   Flashboard 0.1.0 · history 1.2 MB, 8 412 samples           │
│   claude gateway log   ok 12:33                              │
│   codex app-server     ok 12:31                              │
└──────────────────────────────────────────────────────────────┘
```
Settings apply immediately (no Save button); each control shows a brief inline "Saved" tick;
invalid threshold input is corrected on blur with an explanation (warn must be below
critical; range 50..100). The first notification switch is the master (`notifications`, the
same value as the tray's checkbox); the two level switches are disabled, with their values
kept, while it is off. Turning on "Claude live check" opens a confirmation dialog. "Keep
running in the tray" is disabled with an explanation when no tray host exists. "Start at
login" is on by default (1a); if writing or removing the entry fails, the switch returns to
its previous state and says why (`CommandError` `io`). "About and diagnostics" is the About
screen of 1a: the name Flashboard, the version, and the typed diagnostics.

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
shadows except the focus ring and dialogs. **Hit targets**: at least 32 x 32 px for every
pointer target (WCAG 2.5.8 asks for 24 x 24), controls at least 28 px high with the hit area
extended to 32.

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
| `ok` | = `accent` | = `accent` | below warn. An alias of the accent, not a fourth hue: a healthy screen shows neutrals and one blue |
| `warn` | `oklch(56% 0.13 70)` | `oklch(80% 0.13 80)` | warn level (amber) |
| `critical` | `oklch(52% 0.19 30)` | `oklch(72% 0.17 32)` | critical level (vermilion) |
| `unknown` | `oklch(58% 0.006 250)` | `oklch(62% 0.006 250)` | no data |

Status is **never colour alone**: each level also has a shape and a word.

| level | hue | glyph | bar | word |
|---|---|---|---|---|
| ok | accent blue | ● filled circle | solid | none |
| warn | amber | ▲ triangle | solid + threshold tick reached | "High" |
| critical | vermilion | ■ square | diagonal hatch | "Almost out" / "Limit reached" |
| unknown | grey | ○ ring | empty track, dashed outline | "No data" |

The blue/amber/vermilion triple (Okabe-Ito family) stays distinguishable under protanopia,
deuteranopia and tritanopia; shapes and words cover monochromacy. Contrast is tested from the
token file: text ≥ 4.5:1 (target 7:1 for `text`), large numerals ≥ 4.5:1, UI boundaries and
status glyphs ≥ 3:1, in both themes.

**Theme** follows the OS (`prefers-color-scheme`) with a Settings override applied as
`data-theme` on `<html>` before first paint (no flash): the shell puts the saved theme in the
URL fragment when it creates the window (7.3), and a small bundled script, loaded first in
`<head>` (not inline: the CSP forbids that), sets the attribute synchronously. `color-scheme` is set so native
controls match. `forced-colors: active` is supported (system colours, borders instead of
fills).

**Motion.** Durations `fast` 120 ms, `base` 200 ms, `slow` 320 ms; easing
`cubic-bezier(0.2, 0, 0, 1)`; only `transform`, `opacity` and the bar's scale animate.
A bar animates to a new value (`slow`); a changed number cross-fades (`fast`); views
cross-fade (`fast`), no slides. Nothing loops. `prefers-reduced-motion: reduce` removes all
movement and keeps at most a 100 ms opacity change; skeletons become static.

### 9.8 Micro-interactions
- Refresh (⟳): the icon rotates once per fetch (static under reduced motion), the button is
  `aria-busy`; "Updated 12:33" updates in place. It is announced (politely) only after a
  refresh the user started, never for background updates. A second click while busy does
  nothing.
- Copy: the button label becomes "Copied" with a tick for 1.5 s and is announced.
- Threshold change: the tick marks on every bar move live while the slider moves.
- A value that crosses a threshold changes glyph, hue and word together, once, with no
  flashing or shaking.
- Hover or focus on a meter row shows the level-1 detail of 9.1
  ("50.0 % used · resets today 14:00 · status allowed · seen 12:33"), under the rules of
  WCAG 1.4.13 (hoverable, dismissible with `Esc`, persistent).
- Card press: 1 px translate and a `surface-2` flash (`fast`).

### 9.9 Numbers, time and copy
Rules (one implementation per language, shared vectors in `format-vectors.json`):
- **Percent**: whole numbers. `0 < x < 1` → "<1%". `99.5 ≤ x < 100` → "99%". 100 and above →
  the real figure ("100%", "104%"). Never round up to 100.
- **Countdown** (to `resets_at`, from now): ≥ 48 h "3d 4h"; ≥ 1 h "2h 14m"; ≥ 1 min "38m";
  below "under a minute"; past → the `Elapsed` state, never a negative.
- **Absolute time**, local zone, hour cycle from the setting: same day "today 14:00";
  next day "tomorrow 02:00"; within 6 days "Thu 02:00"; else "16 Dec 08:37" (year added when
  it differs). Always available together with the countdown, never one without the other:
  side by side in the detail view; on an overview row the countdown is visible and the
  absolute time is in the level-1 detail and in the row's accessible text.
- **Age**: "just now" (< 1 min), "5m ago", "2h 05m ago", "yesterday 21:10", then a date.
- **Money**: `Intl.NumberFormat` currency style, two decimals, unknown currency as a plain
  number with its code.
- **Window names**: "5-hour", "7-day", "30-day", "N-hour"/"N-day" for rolling windows,
  the raw label in mono for named windows.
- **Tone**: plain, specific, calm. Sentence case. State the fact, then the next step.
  "Not signed in to Codex." not "Authentication error". No exclamation marks, no blame, no
  jargon ("rate limit window" is "window"; "utilization" is "used").

### 9.10 Accessibility (hard requirement, 1a)
WCAG 2.2 level AA is the floor for every screen, state and theme; primary text meets AAA
contrast. Each line below is checked by a named test or by the manual pass, and a failure
blocks the task.

- **Contrast** (1.4.3, 1.4.11): text ≥ 4.5:1 (primary text ≥ 7:1), large numerals ≥ 4.5:1,
  control boundaries, focus ring, bar fill against track, and status glyphs ≥ 3:1, in both
  themes; computed from the token file by a test.
- **Not colour alone** (1.4.1): every level has a glyph shape, a bar pattern and a word (9.7).
- **Keyboard** (2.1.1, 2.1.2): every function, no trap. Tab order equals reading order
  (2.4.3). Shortcuts: `R` refresh, `,` settings, `Esc` back, `1`–`6` open provider, `?`
  shortcut list. Single-key shortcuts act only when no text field has focus and can be
  switched off (`settings.shortcuts`, WCAG 2.1.4). `Esc` and Tab always work.
- **Focus** (2.4.7, 2.4.11, 2.4.13): a 2 px `accent` ring with 2 px offset on
  `:focus-visible`, never removed, never hidden behind the sticky header (scroll padding);
  focus moves to the view title on navigation and returns to the triggering control on back;
  dialogs trap focus and return it.
- **Names and roles** (1.3.1, 4.1.2): one `<main>`, a heading per view and per card; each
  meter is `role="meter"` with `aria-valuenow/min/max` and an `aria-valuetext` that carries
  the full level-1 detail ("50 percent used, resets in 2 hours 14 minutes, today at 14:00,
  status allowed, seen at 12:33"); icon buttons have text names; the chart is described
  (9.4).
- **Status messages** (4.1.3) that are **not spammy**: one polite live region for the whole
  app. It speaks only when (a) a meter changes level (ok, warn, critical), (b) a provider's
  state changes (for example to "can't reach Codex"), or (c) the user's own action finishes
  ("Updated 12:33", "Copied", "Saved"). A number that changes without changing level is
  never announced. Changes within 2 s are merged into one sentence, and the same sentence is
  not repeated within 60 s. Level changes use the wording of the notification (7.6), once.
  `role="alert"` is used only for an error that needs the user's action, once.
- **Pointer** (2.5.7, 2.5.8): targets ≥ 32 x 32 px; nothing needs a drag (sliders have a
  number field and arrow keys; the chart crosshair has arrow keys).
- **Reflow and zoom** (1.4.4, 1.4.10, 1.4.12): usable at 200 % zoom and at a 320 px wide
  window with no horizontal scroll and no clipped text; text respects the system font scale
  and user text spacing.
- **Motion** (2.3.3): everything in 9.7 stops under `prefers-reduced-motion`; nothing
  flashes; nothing loops.
- **Forced colours**: usable under `forced-colors: active`.
- **Disclosure** (1.4.13): the level-1 detail follows the rules in 9.1.

Automated, in CI: axe (`axe-core`) with **zero violations of any impact** on every component
test and on every screen x state x theme in Playwright; the contrast test; keyboard-path
tests (Tab order per screen, every shortcut, focus return); live-region tests (a level
change is announced once; twenty number updates announce nothing). Manual, in the definition
of done (11.5): a keyboard-only walk-through and a screen-reader pass with Orca against a
written script that lists what must be heard on each screen.

### 9.11 Tray
- Icon: a ring gauge drawn at run time (pure function, RGBA, 64 px, cached per 5 %-step and
  level). Fill arc = the headline usage. Level is also a shape: ok = ring only; warn = ring
  plus a centre dot; critical = ring plus a filled centre; no data = dashed ring. A mid-grey
  outline keeps it legible on light and dark panels.
- Linux has no tray tooltip (the StatusNotifier library Tauri uses exposes none), so the
  "tray tooltip" of 1a is the item's title: the tray item's id and title are "Flashboard".
- Menu (Linux tray hosts deliver the menu, not click events, so everything is in the menu):
  ```
  Claude · 5-hour 50% · resets 14:00          (a row opens that provider's detail view)
  Claude · 7-day 37% · resets Thu 02:00
  Codex · 30-day 4% · resets 16 Dec
  Codex · 30-day 4% · as of 12:31             (a stale reading says so instead of the reset)
  ─────────────────────────────
  Open dashboard
  Refresh now
  [x] Notifications
  ─────────────────────────────
  Quit
  ```
  Rows use absolute reset times in a form that stays true without a timer: a reset within
  the next 24 hours is "HH:MM", within 6 days "Thu HH:MM", later "16 Dec" (no "today" or
  "tomorrow", which would go wrong at midnight). Rows are enabled items (disabled items are
  greyed out and hard to read in most hosts). The "Notifications" checkbox is the master
  switch `settings.notifications`. The menu and icon are rebuilt only when their own content
  changes (7.7).
- Tray host: whether one exists is asked **when the answer is needed** (when the window is
  closed, and for Diagnostics), never cached from start-up: at login the app usually starts
  before the panel, and the tray library registers the item by itself when a host appears.
  No host at the moment the window closes: closing quits. Started with `--hidden` and no
  host: the app stays resident anyway (notifications still work) and a second launch opens
  the window.

### 9.12 Notifications
- Only threshold crossings (section 7.6). Title "Claude 5-hour at 80%"; body "Resets today
  14:00." (absolute time only: a notification can stay on screen, and "in 2h 14m" would go
  stale). Application name "Flashboard", `desktop-entry` hint `dev.jev.flashboard`. Warn
  uses normal urgency, critical the critical urgency hint. No sound request, no actions.
  Sending happens on a blocking thread and never stalls the service loop; a failure is a
  `DiagEvent`, nothing else.
- Never: repeated alerts for the same window, alerts for stale data, alerts at start-up for
  a level that was already announced, marketing, tips, "back to normal" messages.

### 9.13 Window behaviour (Hyprland/Wayland)
- The app is resident in the tray. **The web view exists only while the window is open**:
  closing the window hides it and, after 30 s, destroys the web view; opening recreates it
  (target under 400 ms). Hidden cost is therefore the Rust core only.
- Default size 420 x 640, minimum 320 x 400; the last size is saved in the store's `meta`
  table (debounced, 7.5) and restored. Position is the compositor's business on Wayland and is not
  stored. The layout is fluid because a tiling compositor may give any size.
- `jev-dashboard --hidden` starts in the tray (used by autostart). A second launch opens
  the window (creating it if needed) and asks for focus; on Wayland the compositor decides
  whether to give it. A second launch that itself has `--hidden` does nothing.
- **Autostart** (on by default, 1a). `settings.autostart` is the single source of truth and
  the shell reconciles the file with it at every start and on every change: on →
  `~/.config/autostart/dev.jev.flashboard.desktop` exists with
  `Exec=<absolute path of this binary> --hidden` (rewritten when the path changed); off → the
  file does not exist. Pure text generation, atomic write, 0600. Debug builds never write
  it. The first run tells the user and offers the switch (9.5). This session honours XDG
  autostart (verified, 3.4). Documented alternative for a Hyprland session without it:
  `exec-once = jev-dashboard --hidden`. The app never edits Hyprland's configuration; the
  README offers an optional `windowrule` snippet (float, size) for users who want a floating
  panel.
- The app can only find `codex` and `claude` through the `PATH` it was started with. That
  is right on this machine (3.4). Where a session's `PATH` lacks them, the card says
  "Flashboard can't find `codex` on its PATH" and the README explains the fix; the app never
  guesses directories.
- Server-side decorations are left to the compositor; there is no custom title bar.

## 10. Frontend stack

| choice | decision | why |
|---|---|---|
| Framework | **Svelte 5** (runes) + TypeScript strict | Compiles to small direct DOM code, about a third of a React bundle here; fine-grained updates suit a few live numbers; scoped styles; no runtime CSP issues. |
| Build | Vite, static output (no SSR, no router package), `build.assetsInlineLimit: 0` | Tauri serves static files; nothing is inlined as `data:` (8.8). |
| State | One `usage` store (a rune-based class) fed by `get_snapshot` + the `snapshot` event; `settings` store; a tiny in-memory router. Files that use runes outside a component are named `*.svelte.ts` (Svelte 5 compiles runes only there) | The backend owns the state; the UI holds a read model. No state library. |
| IPC | `@tauri-apps/api` `invoke`/`listen`, wrapped once in `src/lib/ipc/client.ts` behind an `IpcClient` interface with a mock implementation for tests | One seam; components never import Tauri. |
| Chart | Hand-written SVG component (`Chart.svelte`, about 250 lines) | Full control of accessibility, theming, step rendering and gaps; no dependency. |
| Styles | Plain CSS with custom properties, scoped per component | No CSS framework, no runtime. |
| Icons | Inline SVG sprites (about 12 icons, own set) | No icon package. |
| Fonts | `@fontsource-variable/geist` and `geist-mono`, latin subset | Local, consistent numerals. |

Budgets: JavaScript ≤ 70 KB gzip, CSS ≤ 12 KB gzip, fonts ≤ 120 KB (checked in CI from the
build output); no request other than the app origin and IPC at run time (Playwright); first
data on screen ≤ 400 ms after window creation on this machine (measured by the smoke test
from `DiagEvent::WindowReady { took_ms }`, which the shell emits when the new web view's
first `get_snapshot` returns).

Frontend tests:
- **Unit** (Vitest): `format`, `copy`, stores, router, chart geometry (pure functions),
  driven by `format-vectors.json`.
- **Component** (Vitest + Testing Library + axe): every component in every state, zero axe
  violations.
- **Screen and visual** (Playwright against the built UI with the mock `IpcClient`, served
  with the production CSP header): each screen x state x theme x width (360, 720, 1200)
  screenshot compared to a committed baseline; axe on each; keyboard-path and live-region
  tests (9.10); reduced-motion and forced-colors runs; one run at device pixel ratio 2
  (hairlines, the chart); a 200 % zoom run at 640 px; "no network" assertion. Browser:
  **Chromium** (present on this machine; Playwright's WebKit does not run on Arch). These
  baselines guard against regressions; they do not prove WebKitGTK renders the same, which
  is what the manual check list of T11 and T13 covers.
- **Real binary**: the driver-free smoke test of 11.4 always; `tauri-driver` +
  `WebKitWebDriver` end-to-end tests in addition when the driver is installed.

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
| integration | service loop with fake clock, fake timer and fake sources: scheduling, backoff, visibility, suspend jump, clock set, alerts once, canary end to end | `cargo test` with `FakeClock`/`FakeTimer` (and tokio paused time for fetch timeouts) |
| component / screen / visual / a11y | section 10 | Vitest, Playwright, axe |
| real binary | smoke test without a driver (always); WebDriver e2e (when the driver exists) | 11.4 |
| security | section 8.10 | all of the above |

### 11.3 Coverage and budgets
- Line coverage (`cargo llvm-cov`): core ≥ 95 %, io ≥ 85 %; the shell is glue and is covered
  by e2e. Vitest: `src/lib` logic ≥ 95 %, components ≥ 85 %.
- Resource budgets, measured by `dashboard/scripts/measure-idle.sh` on the release build,
  hidden and idle for 10 minutes against a temp tree, summed over the **whole process
  tree** (the app and any WebKit helper that is still alive) and over all threads
  (`/proc/<pid>/task/*/`): CPU time < 0.5 s, average wakeups (voluntary context switches)
  < 2 per minute, RSS ≤ 60 MB; binary ≤ 15 MB. That destroying the window really ends
  WebKit's helper processes is assumed (16) and this measurement is what checks it.

### 11.4 `scripts/ci.sh`
One script, fail-fast, prints a table of stages. Stages: (1) engine: `cargo build
--all-targets --locked` and `cargo test --locked` at the root (the 650 existing tests must
stay green); (2) fmt, clippy; (3) dashboard Rust tests incl. property tests at CI case
counts; (4) bindings drift; (5) `cargo deny check`, `cargo audit`; (6) `pnpm install
--frozen-lockfile`, lint, typecheck, unit and component tests (with axe), `pnpm audit
--prod`; (7) UI build and bundle budgets; (8) Playwright screens, visual, accessibility,
keyboard, no-network; (9) release build (`--profile dist`) and manifest snapshot tests;
(10) **real-binary smoke test** (no driver needed): start the `dist` binary in the current
graphical session with `HOME` and every `XDG_*_HOME` pointing at a temp tree that holds a
synthetic `usage.jsonl`, and a `PATH` that contains only stub `codex` and `claude` programs;
read its stderr `diag` lines and assert `Started`, a successful Claude-log and Codex fetch,
`WindowReady` within budget, a new sample within 3 s of an appended row, rows in
`history.db`, no canary anywhere in the temp tree or on stderr, no file outside the temp
tree touched, and a clean exit on SIGTERM; (11) coverage thresholds; (12) fuzz (60 s per
target, nightly); (13) WebDriver end-to-end on the real build (`tauri-driver` +
`WebKitWebDriver`).

Tools: `scripts/ci.sh --bootstrap` installs, at pinned versions, with `--locked`, and without
root: `cargo-deny`, `cargo-audit`, `cargo-llvm-cov`, `cargo-fuzz`, a pinned nightly toolchain
(through `rustup`, which is present), `tauri-driver`, and Playwright's Chromium (already
cached here). A stage whose tool is missing prints **SKIPPED (reason, how to install)**
loudly and the summary lists it; no stage is ever skipped silently. `scripts/ci.sh --release`
treats a skipped stage as a failure, with one exception that is printed as **NOT RUN (needs
root)** and listed in the summary: stage 13, because `WebKitWebDriver` is not packaged with
Arch's `webkit2gtk-4.1` and installing it needs `pacman` (a user decision, section 17). Once
the driver exists, stage 13 is required like the others. Stage 10 needs a graphical session
and is SKIPPED (a failure under `--release`) without one.

### 11.5 Definition of done (per task and for v1)
1. The task's acceptance tests exist and pass; `scripts/ci.sh` is green (no new SKIPPED).
2. No new direct dependency without lead approval; no lint allow without a written reason.
3. No `TODO`, `todo!`, commented-out code or dead code.
4. The task's security rules are each covered by a named test.
5. Public items documented; behaviour changes reflected in this design if they differ.
6. UI tasks: every state of every component implemented and screenshot-tested in both
   themes and three widths; **every item of 9.10 met**: zero axe violations, the contrast,
   keyboard-path and live-region tests green, and a keyboard-only walk-through recorded in
   the report (the keys pressed and what received focus). The three levels of 9.1 are
   present and level 1 is reachable by keyboard.
7. Lead review passed against the task's checklist, and review rounds repeated until a full
   round finds nothing (1a). A task with an open finding is not done.
For v1 as a whole, additionally: `ci.sh --release` green on this machine (stage 13 may be
NOT RUN, 11.4), the idle budgets measured, a manual run beside a live `jev-claude` session
for one day with no gateway interference, the manual check lists of T04, T11 and T13, and a
**manual screen-reader pass with Orca** over every screen and state against the written
script (9.10). Orca is not installed here and needs `pacman`; v1 is not done until the pass
has been made (section 17, Q2).

## 12. Packaging, install, versioning
- Build: `pnpm --dir dashboard install --frozen-lockfile && pnpm --dir dashboard tauri build
  --no-bundle -- --profile dist --locked`. The result is one binary,
  `target/dist/jev-dashboard`, which needs only the system's webkit2gtk-4.1, gtk3 and
  libayatana-appindicator (all present).
- `dashboard/scripts/install.sh [dest]` (default `~/.local/bin`): installs the binary as
  `jev-dashboard`, `dev.jev.flashboard.desktop` (`Name=Flashboard`, `Exec=` the absolute
  installed path) into `~/.local/share/applications`, and icons (named `dev.jev.flashboard`)
  into `~/.local/share/icons/hicolor/{32x32,128x128,scalable}/apps`; `--uninstall` removes
  exactly those, and also the autostart entry the app may have written
  (`~/.config/autostart/dev.jev.flashboard.desktop`), so an uninstall leaves no entry that
  points at a missing binary. It never touches the gateway, the engine binaries or any other
  user configuration.
- `scripts/install.sh` is **unchanged** and keeps installing only the engine tools.
- No AppImage by default (its bundler downloads tooling at build time). An Arch `PKGBUILD`
  is a follow-up.
- Versioning: SemVer, single source `dashboard/src-tauri/Cargo.toml` (`0.1.0` for v1);
  `tauri.conf.json` and the UI read it from there; `dashboard/CHANGELOG.md`. The app version
  is independent of the engine's. Store schema and settings file carry their own integer
  versions with migrations.
- Application identifier `dev.jev.flashboard`; product name "Flashboard"; binary and crate
  name `jev-dashboard` (1a).

## 13. Performance design notes
- Idle hidden: no web view, no timers except the next source deadline (one timerfd), inotify
  blocked in the kernel. The watched directory also holds the gateway's other logs; events
  for other file names are dropped by name in the watcher thread and never reach the service.
- A burst of gateway writes costs one 1.5 s-coalesced tail read of the new bytes only, with a
  cheap byte pre-filter (`"ratelimit":{"`, which a row with an empty `ratelimit` object
  does not contain) before JSON parsing.
- A Codex poll is one short-lived process (about 2.7 s wall, almost all waiting) every
  5/15 minutes; nothing stays resident.
- Snapshots are emitted only on change. Tray icons are cached and the tray is touched only
  when its own content changes. History queries return at most 600 points, aggregated in SQL
  (7.5), so memory per query is bounded by 600 points whatever the range.
- The engine probe (two CLI starts) runs at start and then only when needed (7.2), never on
  a fixed short timer.
- One multi-thread runtime is already owned by Tauri; the service uses it and spawns no
  threads of its own beyond `spawn_blocking` for SQLite and the first-run backfill.

## 14. Risks
| risk | likelihood | mitigation |
|---|---|---|
| `codex app-server` is experimental and its shape changes | medium | allow-list parser tolerant of unknown fields; fixture + schema noted with the CLI version; failure → `Stale{Failing(Protocol)}`, never a crash; a diagnostics line shows it |
| Anthropic renames headers or adds windows | medium | prefix-based generic window parsing; unknown windows render as named meters |
| Claude usage outside the gateway makes the number low | certain | honest "as of" labelling, `Elapsed` state, optional live check |
| Tray host behaviour differs between shells | medium | menu-only interaction model; no-tray fallback decided when the window closes; tested on this machine's host (quickshell) |
| WebKitGTK quirks (clipboard, fonts, compositing on Wayland, `prefers-color-scheme` not following the desktop) | medium | Copy falls back to selecting the text; fonts bundled; the Theme setting overrides the OS; manual check list in T11/T13 on the real web view, because the visual tests run in Chromium |
| `WebKitWebDriver` unavailable on Arch | certain here | Playwright-on-mock covers the UI; the Rust integration suite covers the core; the driver-free smoke test (11.4 stage 10) covers the real binary; WebDriver e2e runs once the user installs the driver |
| `PR_SET_DUMPABLE=0` upsets a portal or WebKit's sandbox helper | low | checked by hand in T11; fallback is `RLIMIT_CORE=0` only (8.8) |
| A session `PATH` without `codex`/`claude` (autostart on another setup) | low here (verified, 3.4) | exact copy on the card, README section, never guess directories |
| A mise command-wrapper for a listed but uninstalled tool starts an installer when the engine probe runs its status command | low | probe is rare (7.2) and the engine kills it after 5 s; a proper fix belongs in the engine's probe (lead item, section 17) |
| The scrubbed child environment breaks `codex app-server` or `claude` (login not found) | medium | allow-list includes the home, bus and tool-home variables (8.6); one real, free Codex run under the scrubbed environment in T04's manual list |
| `notify-rust`/`zbus` API or version drift | low | pinned; one module each; the Notifier and tray-host checks are behind traits with fakes |
| The engine library drags heavy dependencies into the app | certain, low impact | link-time removal; split `jev-engine` later |
| Web view memory when open (WebKit processes, 100 MB+) | certain | web view destroyed when the window is closed; that its helper processes exit is measured by `measure-idle.sh` over the process tree |
| DeepSeek adapter unverified until a key exists | certain | built against the documented shape with fixtures; marked unverified in Diagnostics until first success |
| Shared `Cargo.lock` churn for the engine | low | `default-members`, `--locked`, engine tests in stage 1 |

## 15. Designing for the later chat window (not built)
- The engine is reached only through `jev_router::embed`, which already exports `Runtime`;
  the chat feature adds turn execution exports there and a `jev-chat-io` crate beside
  `jev-usage-io`. The usage service is unaffected.
- Turns run by the chat engine yield `PlanUsage`/rate-limit events; a `ChatTurnSource`
  implementing `UsageSource` with `Trigger::OnSignal` keeps the latest event and signals the
  service through the same `signals` channel the file watcher uses, so Claude usage becomes
  live while chatting with no new UI and **no change to the trait, the service or the IPC**.
- `Notifier`, `HistoryStore`, `SettingsStore`, `WallTimer` and `EngineProbe` are unaffected by
  chat. `Settings` gains fields through its `version` and migration, not a new store.
- The IPC rule changes for chat (prompts and answers must cross). That will be a **separate
  window label with a separate capability file and separate commands**, so the dashboard
  window's "numbers only" guarantee and its tests stay intact.
- The shell's window manager already creates and destroys windows by label.

## 16. Verified versus assumed
**Verified** (section 3): Codex app-server transport, sequence, timings, exit behaviour and
free-plan shapes; `usage.jsonl` field names, value types, freshness and line size; tray and
notification hosts present; Tauri CLI `info` output; toolchain versions; which tools are
missing; `gemini --help` has no usage command; engine probes never run `grok`/`agy`;
`ts_ms` order, the `ratelimit` key set and the sporadic `7d_oi` window (3.2); log rotation by
rename; XDG autostart is honoured by this session and its `PATH` finds `codex` and `claude`
(3.4); what the engine's probe reads and runs (5.1).

**Assumed**: paid Codex plans report 300- and 10080-minute windows (schema only);
`account/rateLimits/updated` is not pushed without turns; meaning of Claude's `7d_oi`
window; DeepSeek balance response shape (documentation); `claude -p` emits a
`rate_limit_event` on a minimal request and its exact cheapest flags; Antigravity headless
`/usage` JSON shape; xAI Management API shape; WebKitGTK allows `navigator.clipboard.writeText`
under the app origin; current latest stable versions of Tauri 2, Svelte 5 and Vite (the
scaffold task pins what it finds); `tauri build -- --profile dist` puts the binary in
`target/dist/`; `codex app-server` and `claude` work under the scrubbed environment of 8.6;
Codex `usedPercent` may be fractional and `credits.balance` is a decimal string;
`tauri-plugin-notification` has no urgency setting on Linux while `notify-rust` does;
destroying the window ends WebKit's helper processes; `incognito(true)` leaves no WebKit
directory on disk; WebKitGTK follows the desktop's light/dark preference here;
`PR_SET_DUMPABLE=0` does not disturb portals or WebKit; `WebKitWebDriver` from Arch's
`webkitgtk-6.0` package can drive a `webkit2gtk-4.1` app. Each of these has a named check in
the task that depends on it, and none blocks the design if it turns out false (the fallback
is written where the assumption is used).

## 17. Open questions
Answered by the user on 2026-11-17 (section 1a): the name, the Claude live check, start at
login, the account e-mail, what comes after v1, and the UI quality bar.

Still for the user (none blocks the start of implementation):
- Q1. `WebKitWebDriver` is not installed and needs root: install it (`sudo pacman -S
  webkitgtk-6.0`, assumed to provide the driver) so the WebDriver end-to-end stage can run,
  or accept v1 with the driver-free smoke test and that stage reported as NOT RUN (11.4)?
- Q2. The manual screen-reader pass (1a, 11.5) needs Orca, which is not installed and needs
  root (`sudo pacman -S orca`). May it be installed, or will the user run the pass?
- Q3. The interrupted `antigravity-cli@1.5.9` download left an empty directory under
  `~/.local/share/mise/installs/antigravity-cli/`: remove it or let `agy` finish installing?

For the lead (resolved during implementation, not user-owned): split of `jev-engine` from
`jev-router`; a narrower engine probe for embedders (one provider at a time, scrubbed
environment, no installer start through a mise wrapper, no token file read: see 5.1);
whether to store a last reading for instant cold-start display; PKGBUILD.
