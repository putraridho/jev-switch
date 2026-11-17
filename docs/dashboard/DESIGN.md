# Flashboard (the Jev usage dashboard): design

Status: design for v1. The user's decisions are recorded in section 1a; section 17 has no open user question. Normative for every task in `TASKS.md`.
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
 jev-usage-core (domain, parsers, policy; pure, no IO)        the engine, behind ONE port
                                                             (io::engine_bridge, 5.1)
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
  tasks T07, T09, T10, T13 and T14 implement this.
- **Type safety ("strictest codebase ever"):** every type is defined; nothing is typed
  loosely, in either language, in hand-written **or generated** code.
  *TypeScript*: no `any`, `unknown` or `never` written anywhere; no type assertion (`x as T`,
  `<T>x`; `as const` and `satisfies` are not assertions and are allowed); no non-null `!`; no
  `@ts-ignore`, `@ts-expect-error` or `@ts-nocheck`; no `Function`, `Object`, `object` or `{}` types;
  no `catch` binding (so no `unknown` error value ever exists: failures arrive as typed
  `Outcome` values from the IPC layer and the UI never catches); exhaustiveness through
  `satisfies Record<Union, …>` tables and the switch-exhaustiveness lint, never a `never`
  check; every Svelte component is `lang="ts"` with typed props; all IPC types are generated
  from Rust. *Rust* (the three app crates): no `serde_json::Value` or `json!` outside tests
  (typed structs and enums at every boundary), no `unwrap`/`expect` outside tests,
  `#![forbid(unsafe_code)]` in all three crates, strict clippy in CI. Sections 7.3, 7.4, 10
  and 11.1 specify it; T00 and T12 enforce it.
- **WebDriver end-to-end tests:** the user installs the driver themselves
  (`sudo pacman -S webkitgtk-6.0`; that Arch package ships `/usr/bin/WebKitWebDriver`;
  `webkit2gtk-4.1` ships no binary). **Done**: `/usr/bin/WebKitWebDriver` is present and
  owned by `webkitgtk-6.0 2.54.1-1` (verified with `pacman -Qo`, 3.4). The real-app
  WebDriver stage is therefore **in scope and required**: it must pass in `ci.sh --release`.
  On a machine without the driver the stage reports **NOT RUN**, which is never a pass and
  never satisfies the definition of done (11.4, 11.5).
- **Screen-reader pass:** the user installs Orca (`sudo pacman -S orca`; **done**:
  `orca 51.1-1` is installed, 3.4) and takes part in
  the manual pass: the lead writes the script of checks per screen and state, the user
  confirms what Orca announces. That pass is part of the definition of done (11.5).
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
- `codex` and `claude` on this machine are mise command-wrappers: symbolic links to
  `/usr/bin/mise` in `~/.local/share/mise/command-wrappers/bin` (first on `PATH`). The
  system mise configuration declares both tools `lazy = true, auto_update = "4h"` and routes
  them through `omarchy-agent-account-exec`, which picks the active subscription's home
  (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`) and then starts the real binary under
  `~/.local/share/mise/installs/<tool>/<version>/` (`claude/<version>/claude`,
  `codex/<version>/bin/codex`; names only were listed). So **running either name can start a
  download** (first-run install, or the four-hourly self-update), and bypassing the wrapper
  could read the wrong account. Section 8.6 handles both. mise maps settings to environment
  variables (verified: `MISE_OFFLINE=1`, `MISE_AUTO_INSTALL=false`,
  `MISE_EXEC_AUTO_INSTALL=false`, `MISE_NOT_FOUND_AUTO_INSTALL=false` are reported back by
  `mise settings get`, mise 2026.11.8). mise's documentation (read 2026-11-17, not run):
  `offline` "blocks every HTTP request, so mise uses only installed tools and cached data";
  the per-tool `auto_update` runs `mise upgrade` for that tool **before** the tool starts
  when a check is due, and "no updates run offline"; `auto_install = false` already implies
  the other two auto-install switches (they are set anyway: harmless and explicit).
- **The account wrapper has its own, explicit install path** (read from
  `/usr/share/omarchy/bin/omarchy-agent-account-exec`, 56 lines; not run): mise removes its
  dispatch directories from `PATH` and calls the script; when `type -P <tool>` then finds no
  real binary, the script runs `mise install --quiet <tool>`, or `mise use -g --quiet <tool>`
  when the tool is not one Omarchy declares, **which rewrites the user's global mise
  configuration**. Neither is an "auto install", so the three auto-install switches do not
  stop them; only `MISE_OFFLINE=1` blocks their download, and nothing blocks the
  configuration write of `mise use -g`. The script also calls `omarchy-agent-account-home`
  (found through `PATH`; reads `XDG_STATE_HOME`/`HOME`) and ends in `exec`, so the real tool
  stays in the caller's process group. Consequence for 8.6: a tool counts as installed only
  when its **executable** is on disk, which is what keeps this fallback unreachable.
- `account/usage/read` returns `{"summary":{"lifetimeTokens":0,"peakDailyTokens":0,
  "longestRunningTurnSec":null,"currentStreakDays":0,"longestStreakDays":0},
  "dailyUsageBuckets":[],"threadUsage":null}`. Not needed for v1.
- `account/read` returns e-mail, `userId` and `chatgptAccountId`. **The dashboard MUST NOT
  call it**: the plan name is already in `rateLimits.planType`.
- `account/rateLimits/updated` exists (sparse rolling update) but is only expected during
  turns run by that same server. The dashboard runs no turns, so it polls with a short-lived
  process instead of keeping a server resident.
- Protocol schema: `codex app-server generate-json-schema --out DIR`.
- `codex login status` prints `Logged in using ChatGPT` on **stderr** (stdout is empty) and
  exits 0. `claude auth status` prints a 16-line JSON object on stdout whose keys include
  `loggedIn`, `planTier`, `subscriptionType` and also `email`, `orgId`, `orgName` and
  `configDirectory` (key names checked; values not recorded).

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
  | `5h-status`, `7d-status` | `"allowed"`, `"allowed_warning"`, `"rejected"` | window status |
  | `5h-surpassed-threshold` | `"0.9"` (only with `allowed_warning`) | not used in v1 |
  | `status`, `reset` | `"allowed"`, `"1794898800"` | overall; not used in v1 |
  | `fallback`, `upgrade-paths` | `"available"`, `"upgrade_plan,overage"` | not used in v1 |
  | `representative-claim` | `"five_hour"` | which window binds |
  | `fallback-percentage` | `"0.5"` | not used in v1 |
  | `overage-status`, `overage-disabled-reason` | `"rejected"`, `"org_level_disabled"` | not used in v1 |
  | `slow-offer`, `slow-max-wait`, `slow-retry-after` | on 3 rows | not used in v1 (no attribute the key rule of 6.1 reads) |
| `7d_oi-utilization/-reset/-status` | on 45 of 10 528 rows (88 of 11 051 at the second check), never on consecutive runs | an extra window; meaning unknown; **present only on some rows** |

  A `retry-after` key without the prefix also occurs (3 rows) and is ignored.

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
- Installed by the user since the first spike (verified 2026-11-17 with `pacman -Qo`):
  `/usr/bin/WebKitWebDriver` (`webkitgtk-6.0 2.54.1-1`; `webkit2gtk-4.1` ships none; both
  packages are WebKitGTK 2.54.1) and `/usr/bin/orca` (`orca 51.1-1`). `dbus-run-session`
  and `desktop-file-validate` are installed. Still missing for stage 13: `tauri-driver`
  (no root needed, `--bootstrap`). Section 11 handles each.

### 3.5 Incident during the spike (disclosed)
Running `agy --help` under a 15 s timeout started a mise download of `antigravity-cli@1.5.9`
(the brief warned only about `grok`; `agy` is the same kind of lazy shim). The timeout killed
it at about 7 of 99 MB. It left an empty directory
`~/.local/share/mise/installs/antigravity-cli/1.5.9`, which the lead has since removed. Rule
added to the design: **never execute `agy`, `grok` or `gemini`; never start `codex` or
`claude` unless a real install exists on disk; and switch mise's installers off in every
child's environment** (section 8.6).

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
/// ("max_5x", "free") and unknown window names ("7d_oi"). It cannot hold a prompt, an
/// e-mail, a path or a URL (no space, `@`, `/`, `.` or upper case). It CAN hold 40
/// characters of lower-case identifier, so the rule that protects identity is positional:
/// a `Label` is only ever read from a field that names a plan or a window (8.3).
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
The mapping to the engine's `ProviderId` lives inside `jev-usage-io::engine_bridge`
(`engine_bridge::login_command(ProviderKey)`, the one v1 use: the engine's login command
takes a `ProviderId`), so core does not depend on the engine and no engine type appears in
any Flashboard signature (5.1).

### 4.2 Meters and readings
```rust
// meter.rs
#[derive(Clone, Eq, PartialEq, Hash, Debug)]                                              // IPC
pub enum WindowKind { FiveHour, SevenDay, Monthly, Rolling { minutes: u32 }, Named { label: Label } }
impl WindowKind { pub fn from_minutes(m: u64) -> Self; }   // 300, 10080, 43200 map to the named kinds

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum MeterId { Window(WindowKind), Balance(Currency) }
impl MeterId {
    /// Stable storage/IPC key: "w:5h" "w:7d" "w:30d" "w:r:<minutes>" "w:n:<label>" "b:usd" "b:cny" "b:other".
    pub fn key(&self) -> String;
    pub fn parse_key(s: &str) -> Option<Self>;             // exact inverse of key()
}
// MeterId serialises as its key (hand-written Serialize/Deserialize through key()/parse_key()),
// so it is the type of every `id`/`meter_id` IPC field and no `String` field is needed.

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
pub enum StaleCause { Quiet, Failing { reason: Unavailable } }  // Quiet: passive source saw nothing new
#[derive(Copy, Clone, Eq, PartialEq, Debug)]                                              // IPC
pub enum Unavailable { NotInstalled, NotLoggedIn, Offline, Timeout, RateLimited,
                       Unauthorized, ServerError, Protocol, Io, Internal }
#[derive(Copy, Clone, Eq, PartialEq, Debug)]                                              // IPC
pub enum Setup { UseGateway, AddDeepSeekKey }
```
`Named` and `Failing` are struct variants because the IPC enums are internally tagged
(7.4) and a tagged newtype variant does not serialise as `{kind, field}` (verified: with a
string inside it is an error, with an enum inside it produces a wrong shape); the prose of
this document writes `Named(x)` and `Failing(x)` for short.
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
older than `fresh_for` with no newer failure → `Stale{Quiet}`. Only
`SourceError::Unavailable` is a failure: `NotConfigured` and `Unsupported` are answers, they
are shown only while the provider has no reading and never displace one. A failure of a
`Trigger::Manual` source (the live check) is never written to `last_failure` when the
provider holds a reading: it is reported as `RefreshView.live_check_error` (7.4), so a failed
check does not make the passive reading look broken. `fresh_for` is the
`SourceDescriptor.fresh_for` of the source that produced `last_good`. **Clock skew**: ages are
saturating differences, and the service clamps a reading's `observed_at` to `now` when it
arrives from the future (a gateway row stamped ahead of the wall clock, or a clock set
back), so a reading can never look fresh for longer than its policy and no sample is stored
with a future time.

### 4.4 Ports (traits core defines, io implements)
```rust
// port.rs
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]                                        // IPC
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
#[derive(Copy, Clone, Eq, PartialEq, Debug)] pub enum Cost { Free, SpendsQuota }
#[derive(Copy, Clone, Debug)]
pub struct SourceDescriptor { pub id: SourceId, pub provider: ProviderKey,
                              pub trigger: Trigger, pub cost: Cost, pub timeout: Duration,
                              /// How long a reading of this source counts as current (7.7).
                              pub fresh_for: Duration,
                              /// True when the source starts a CLI that needs a login: the
                              /// service probes the login first when it is not known `Ready` (7.2).
                              pub needs_login: bool }
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
    /// Points, reset marks, bucket width. A span (`to - from`) longer than
    /// `history::RAW_RETENTION` is a rollup query (7.5); the store is not told the range name.
    fn series(&self, provider: ProviderKey, meter: &MeterId, from: UnixSeconds, to: UnixSeconds,
              max_points: u16) -> Result<SeriesData, StoreError>;
    fn load_alerts(&self) -> Result<AlertMemory, StoreError>;
    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError>;
    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError>;
    fn stats(&self) -> Result<StoreStats, StoreError>;      // file bytes and sample count, for Diagnostics
}
pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Settings;                              // defaults when missing or invalid
    fn save(&self, s: &Settings) -> Result<(), StoreError>;
}
/// MUST NOT block: an implementation hands the alert to its own worker and returns.
pub trait Notifier: Send + Sync { fn notify(&self, alert: &Alert, hour: HourCycle); }   // the service passes the current setting
```
`HistoryStore` and `SettingsStore` are synchronous and do file IO; the service calls every
method of both through `spawn_blocking`.
A provider may have several sources (Claude: gateway log and the opt-in live check). The
service keeps the reading with the greatest `observed_at`; a reading **strictly** older than
the one it holds is dropped, and one of equal age replaces it (that is how a plan name that
the login probe learned after the first log read reaches the card: when a probe changes a
provider's facts, the service fetches that provider's `OnSignal` sources once more).

### 4.5 Policy (pure functions in core)
- `assess::derive(state, now, policy) -> Usage` and `assess::phase(window, now) -> Phase`
  (`Current` or `Elapsed` when `resets_at <= now`).
- `assess::severity(&Meter, now, &Thresholds) -> Severity`: a window meter is `Ok`, `Warn` or
  `Critical` by its `used`, and `Unknown` when it is elapsed; a balance meter is `Critical`
  when `usable` is false, else `Ok`. (It takes the meter, not a bare percentage: a
  percentage alone cannot say "elapsed" or "balance".)
- `alerts::step(memory, provider, reading, now, settings) -> (AlertMemory, Vec<Alert>)`:
  section 7.6.
- `summary::headline(&[(ProviderKey, &Usage)], now, &Settings) -> Option<summary::Headline>`:
  the window meter with the highest `used` among `Known` and non-elapsed `Stale` readings of
  enabled providers. `summary::Headline` is the domain value (`MeterId`, `Percent`);
  `view::snapshot` maps it to the IPC `Headline` of 7.4.
- `history::should_store(last_stored, candidate) -> bool`, `history::downsample(points, max)`,
  `history::reset_marks(sorted_resets, kind) -> Vec<UnixSeconds>` (collapses a sliding
  `resets_at` into one mark per window instance: a new mark needs a jump of at least half the
  window length, or 1 hour for a window of unknown length; at most 64 marks),
  `history::gap_for(bucket) -> Duration` (`max(75 min, 2.5 x bucket)`: the longest hole the
  chart bridges; 75 min is 2.5 x the longest polling interval, the same figure as the
  longest `fresh_for`).
- `text::*`: the only place tray and notification strings are built (section 9.9).
- `settings::Settings::validated(raw) -> Settings`.

### 4.6 SOLID mapping
| principle | how it is met |
|---|---|
| Single responsibility | One module, one reason to change: each `parse::*` knows one wire format; `assess` knows freshness; `alerts` knows thresholds; `tail` knows files; `store` knows SQL; `tray` knows the tray. |
| Open/closed | New provider = new `UsageSource` + one registry line + one `ProviderKey` variant. The service, store, IPC and UI render from `Meter` generically. |
| Liskov | Every `UsageSource` returns the same `Reading`/`SourceError`; the service never matches on `SourceId` to behave differently. Test: the service test-suite runs against a fake source and every real adapter driven by fixtures. |
| Interface segregation | Four small ports in core (`UsageSource`, `HistoryStore`, `SettingsStore`, `Notifier`) plus `Clock`; three in io for its own IO seams (`WallTimer`, `LoginProbe`, `Jitter`). The UI gets one read model (`Snapshot`). |
| Dependency inversion | `core` defines ports and depends on no IO, runtime or engine crate (its whole list is in 8.9). `io` implements them. The Tauri shell wires them. Nothing in core or io knows Tauri exists. |

DRY rules: one parser per wire format; one formatter per language with shared test vectors
(`dashboard/fixtures/format-vectors.json`, consumed by Rust `text` tests and TS `format`
tests); TypeScript IPC types are generated from Rust, never hand-written; engine facts
(state dir, env file path, env parsing, the Codex login-status rule, the login commands) come
from the engine through the one port `io::engine_bridge` (5.1). One deliberate exception, for privacy: the dashboard has its own
allow-list parser for `claude auth status` (5.1), because the engine's parser builds a
generic JSON value of the whole output, e-mail included.

## 5. Repository and crate structure

**Decision: the three app crates are members of the repository's one Cargo workspace and
are never default members.**

The engine is about to be refactored into a workspace with one crate per bounded context
and a dedicated facade crate (`docs/architecture/DDD.md`, on its own branch, still under
review). **Ordering assumption: T00 may start before or after that refactor, and both
orders are valid.** Everything below is therefore written as invariants that hold either
way, plus the two concrete forms:

| | **Path A**: T00 runs before the refactor | **Path B**: T00 runs after it |
|---|---|---|
| `[workspace]` table | T00 creates it (block below) | exists; T00 only appends the three members and the two `exclude` entries |
| default members | `["."]` (the engine package) | the engine's members only. The refactor's root is virtual and, as planned, has **no** `default-members` key (then every member is a default member), so T00 **adds** `default-members = ["crates/*"]` when the key is absent, in the same commit as the three members (the refactor's plan, section 4.4, says the same); when the key exists T00 leaves it alone |
| engine facade | the module `jev_router::embed`, created by T00 (5.1) | the refactor's facade crate; T00 creates no engine file |
| what the refactor / T00 edits in Flashboard | later, the refactor changes the **two lines** of 5.1 and nothing else | T00 writes those two lines for the facade crate |

Invariants (review-blocking in both paths):
1. The three app crates are workspace members and **never** in `default-members`, so the
   engine's root commands (`cargo build`, `cargo test`, `scripts/install.sh`) never build
   or test Flashboard. Building the app is always explicit (`-p jev-dashboard`).
2. Flashboard reaches the engine through **one port** (`io::engine_bridge`, 5.1). The
   engine's crate name and path appear in exactly two lines of the whole app.
3. Flashboard does **not** use `[workspace.lints]` or a root `clippy.toml`: each app crate
   carries the same `[lints]` table in its own manifest (a test compares the three), and
   the clippy configuration is `dashboard/clippy.toml`. The engine's refactor owns the
   workspace-level lint table alone, and neither side can break the other's lint policy
   (the two policies differ: the engine needs one `unsafe` file and attribute reasons;
   Flashboard forbids `unsafe` and uses the fixed test header of 11.1). Known conflict
   with the refactor's plan as it stands (its 4.4 and check C1 expect every member to say
   `[lints] workspace = true`, to write every dependency as `workspace = true` and to sit
   under `crates/`): in Path B the three app crates need an exemption from that check, a
   request to the engine's owner recorded in T00's report, like invariant 5.
4. `[profile.dist]` is Flashboard's and is used by nothing else; no profile sets `panic`.
5. No engine gate may compile the app crates by accident: they need webkit2gtk **and** the
   built UI (invariant of 11.4), so an engine command that says `--workspace` must exclude
   the three (`--exclude jev-usage-core --exclude jev-usage-io --exclude jev-dashboard`).
   In Path B this is a request to the engine's owner, recorded in T00's report (the
   plan's gate script runs `cargo clippy --workspace` and `cargo test --workspace` today).
6. T00 states in its report which path it took.

```toml
# Cargo.toml (root), Path A: the ONLY additions. Path B: only the marked lines are added,
# plus `default-members = ["crates/*"]` when the refactor's root has no such key (table above).
[workspace]
members = [".", "dashboard/crates/usage-core", "dashboard/crates/usage-io", "dashboard/src-tauri"]   # B: append the three
default-members = ["."]
exclude = [".claude", "dashboard/fuzz"]   # see below                                                # B: append both
resolver = "2"

[profile.dist]           # used only by the dashboard release build                                  # B: add
inherits = "release"
lto = "thin"
codegen-units = 1
strip = true
# panic stays "unwind": see below
```
`panic = "abort"` is deliberately **not** set. The app is resident and nothing restarts it
(the autostart entry runs once per login), so a panic in one fetch task must end that task,
not the tray: with unwinding the runtime turns it into a failed join, which the service
reports as `Unavailable(Internal)` (the T08 test "a panicking source cannot stall the loop"
is only true this way). The panic hook (8.4) prints the same base name and line either way.
Honest limit: a panic on the GTK main thread, or one that crosses a C callback, still ends
the process.

Why a workspace member and not a sibling repository or a separate nested workspace:
- **DRY, one lockfile**: engine and app resolve the same `tokio`, `serde`, `reqwest`; one
  `cargo deny`/`cargo audit` pass covers both; one `target/` avoids compiling shared
  dependencies twice.
- **Root commands unchanged** (invariant 1): `cargo build --all-targets`, `cargo test` and
  `scripts/install.sh` (`cargo build --release --bin ...`) at the root select only the
  engine's packages, exactly as before Flashboard existed. With resolver 2, features are
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
Cargo.toml                         root manifest (+ workspace members, exclude, profile.dist)
src/embed.rs                       Path A only: engine facade module (new, re-exports only, 5.1)
src/availability.rs                Path A only: one word changes, `pub` on `parse_codex_login`
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
  clippy.toml                      disallowed types/macros and the in-test switches (11.1); here, not at the root,
                                   so it applies to the three app crates and never to the engine (invariant 3)
  fuzz/                            cargo-fuzz targets (stage 12, needs nightly; excluded from the workspace)
  e2e/                             Playwright (mock IPC), the real-binary smoke test, and the required WebDriver tests
  scripts/                         install.sh, measure-idle.sh, check-types.ts
  package.json, pnpm-lock.yaml, .npmrc, vite.config.ts, vitest.config.ts, playwright.config.ts, tsconfig.json,
  eslint.config.ts, .stylelintrc.json, .prettierrc.json   (no svelte.config file: section 10)
```

### 5.1 The engine port: `io::engine_bridge`
Flashboard needs five small facts from the engine and nothing else. It owns the **port**
(the functions it calls, in its own types); the engine supplies a **facade** behind it.

```rust
// jev-usage-io::engine_bridge: the ONLY file that names the engine. No engine type appears
// in any signature; `ProviderId` is mapped inside `login_command`.
pub fn state_dir() -> Option<PathBuf>;                              // the gateway's state directory
pub fn env_file() -> Option<PathBuf>;                               // ~/.jev-router.env or its override
pub fn parse_env(text: &str, name: &str) -> Option<String>;         // pure: one key of env-file text
pub fn codex_logged_in(exit_ok: bool, text: &str) -> Option<bool>;  // pure: the engine's rule for `codex login status`
pub fn login_command(p: ProviderKey) -> Option<&'static str>;       // "codex login", "claude auth login"; None where the engine has none
// Flashboard's own, built on the five above (no further engine call):
pub fn usage_log_path() -> Option<PathBuf>;   pub fn dashboard_dir() -> Option<PathBuf>;
```
It is a module of plain functions, not a trait: all five are pure or only read the
environment, tests call them directly, and an injectable trait would add a seam nobody
needs. Every other Flashboard module (`envfile`, `probe`, `secrets`, the shell, the tests)
calls `engine_bridge::*` and never the engine.

**The two lines.** The engine's crate name and path are written in exactly two places:
```toml
# dashboard/crates/usage-io/Cargo.toml: the one dependency line (renamed, so code says `jev_engine`)
jev-engine = { package = "jev-router", path = "../../.." }           # Path A
jev-engine = { package = "<facade crate>", path = "<its path>" }      # Path B, names from DDD.md when final
```
```rust
// dashboard/crates/usage-io/src/engine_bridge.rs: the one import line
use jev_engine::embed as engine;    // Path A (a module of the engine crate)
use jev_engine as engine;           // Path B (the facade crate's root)
```
`usage-core` and the shell do not declare the dependency at all, so the compiler, not a
convention, keeps them off the engine. The architecture test adds: the identifiers
`jev_engine` and `jev_router` occur in no `.rs` file under `dashboard/` except
`engine_bridge.rs`, and in no `Cargo.toml` except `usage-io`'s.

**What the facade must offer** (the contract between the two projects; it may offer more,
for example for the later chat window, and Flashboard ignores the rest):
`ProviderId`, `state_dir() -> Option<PathBuf>`,
`env_file_path(Option<String>, Option<String>) -> Option<PathBuf>`,
`parse_env(&str, &str) -> Option<String>`, `parse_codex_login(bool, &str) -> Option<bool>`,
`login_command(ProviderId) -> Option<&'static str>`. All exist in `src/` today with these
signatures (checked: `log::state_dir`, `envfile::env_file_path`, `envfile::parse`,
`availability::parse_codex_login`, `doctor::login_command`); only `parse_codex_login` is
not yet `pub`.
- *Path A*: T00 adds `src/embed.rs` (the six `pub use` lines, **re-exports only**), one
  `pub mod embed;` line and the word `pub` on `parse_codex_login`. Nothing else in the
  engine changes; its behaviour and tests are unchanged.
- *Path B*: the facade crate already exists. If it lacks one of the six names, that is a
  request to the engine's owner, never a T00 edit of an engine crate. Known today: the
  plan's facade list has no `parse_codex_login` (and still lists `probe_all` and
  `path_from_env`, which Flashboard does not use), so that one request is certain.
- **Contract tests** (`usage-io/tests/engine_bridge.rs`, T00) pin the *behaviour* Flashboard
  relies on, so a refactor that changes it fails here and not in a user's tray: the
  env-file cases (`export`, quotes, comments, missing key), the Codex login rule's truth
  table, the two exact login command strings, the state directory under a temp
  `XDG_STATE_HOME`/`HOME`, and the env-file path with and without its override. The
  cases that need a different environment (the facade reads the process environment
  itself) run in a **child process**: the test re-executes its own test binary with
  `Command::env`. No test calls `std::env::set_var` or `remove_var`: they race with the
  parallel test harness, and are `unsafe` from edition 2024 on, which 1a forbids.
- **Ripple check**: moving from Path A to Path B is a diff of the two lines above (plus the
  root manifest, which the refactor owns and must carry over: the three members,
  `default-members` without them, the two `exclude` entries and `[profile.dist]`). If it needs more, the port has leaked
  and that is a design defect to fix here, not to absorb.

`state_dir()` **creates** `~/.local/state/jev-router` when it is missing. Harmless (the
gateway would create the same directory) and, after `hardening::apply`, created 0700.

**What the dashboard deliberately does not call**, and why (read from `src/availability.rs`,
`src/log.rs`, `src/envfile.rs` and observed on this machine):
- `availability::probe_all()` probes all five engine providers. It runs `claude auth status`
  and `codex login status` with the caller's **whole environment and working directory**;
  its `kill_on_drop` reaches only the direct child, so a launcher's grandchild (a mise
  installer) survives the 5 s cap; it turns the whole `claude auth status` output (e-mail,
  org id and org name included, 3.1) into a generic JSON value; it reads
  `~/.codex/auth.json` and decodes the `id_token` claims, reads `~/.codex/models_cache.json`
  and `~/.gemini/settings.json`, checks `~/.gemini/oauth_creds.json`,
  `~/.gemini/antigravity-cli` and `~/.grok`, and looks every provider key up through the env
  file. None of that is acceptable for a resident app that promises 1a and 8.3.
- `log::path_from_env` reads the whole env file into an ordinary `String`.

Instead the dashboard has **its own narrow login probe** (`io::probe::CliProbe`, specified
in 8.6) and its own zeroizing env-file reader (`io::envfile`), and takes only pure rules
from the engine. The usage-log path is: the `JEV_USAGE_LOG` process variable, else that key
in the env file (read by `io::envfile`, parsed by `engine_bridge::parse_env`), else
`engine_bridge::state_dir()/usage.jsonl`: the same order as the engine's `path_from_env`,
proven equal by a test over a temp home.

Known cost, Path A only: `jev-router` as a library pulls `axum` and `rustyline` into the
app's build (unused code is dropped at link time). The refactor's facade crate is the fix;
Flashboard does nothing about it.

### 5.2 Module ownership
```
jev-usage-core            jev-usage-io                     jev-dashboard (shell)
  time quantity label       engine_bridge  (the engine port) app       (wiring only; lib.rs lists modules, main.rs calls run)
  provider meter usage      clock          (SystemClock)     commands  (6 commands)
  port                      fsutil         (0600, atomic)    events    (emit snapshot)
  secret                    diag           (typed log)       tray      (menu + icon swap)
  parse::claude_log         envfile        (zeroizing read)  tray_icon (pure rasteriser)
  parse::claude_event       tail, watch    (file tail)       notifier  (Notifier impl)
  parse::claude_auth        proc           (subprocess)      window    (create/hide/destroy)
  parse::codex              probe          (login probe)     autostart (XDG entry text)
  parse::deepseek           http           (the one client)  hardening (umask, dumpable, panic hook)
  assess alerts summary     secrets        (key loading)     bus       (the one D-Bus user: tray host)
  history text settings     sources::{claude_log, claude_live, codex, deepseek, placeholder}
  view ipc                  store          (SQLite)          localtime (LocalTime impl)
                            settings_file                    cli       (--hidden)
                            timer          (wall-clock timer)
                            registry       (source list)
                            service        (the one loop)
```

## 6. Sources (adapters)

| source | trigger | cost | reads | never touches |
|---|---|---|---|---|
| `ClaudeGatewayLogSource` | signal: file change on `usage.jsonl` | free | `ts_ms` and the allow-listed `ratelimit` keys | every other key; `gateway.jsonl`, `turns.jsonl`, `history`, `sessions/` |
| `ClaudeLiveSource` | manual, opt-in, off by default | **spends a tiny request** | `rate_limit_event` lines of one `claude -p` run | the answer text (discarded unparsed) |
| `CodexAppServerSource` | interval 5 min visible / 15 min hidden | free | `account/rateLimits/read` result | `account/read`; any file under `~/.codex` |
| `DeepSeekBalanceSource` | interval 10 min visible / 30 min hidden | free | `GET /user/balance` body | anything else on the network |
| `PlaceholderSource` (Gemini, Antigravity, Grok) | never | free | nothing | everything (returns `Unsupported`) |

### 6.1 Claude gateway log
- Path from `engine_bridge::usage_log_path()` (5.1: the engine's rule, re-stated without its
  file reader), resolved once at start;
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
  `parse_line` first applies the byte pre-filter `might_have_sample` (the bytes
  `"ratelimit":{"`) and returns `None` when it fails: the gateway writes compact JSON
  (`serde_json::Value::to_string` in `src/log.rs`, verified), so a row with other spacing
  is not a gateway row.
- **Key rule** (exact): strip the prefix, then split the rest at its **last** `-` into
  `(window, attribute)`. Only the attributes `utilization`, `reset` and `status` are read.
  A key with no `-` (`status`, `reset`) has no window and is ignored. So
  `5h-surpassed-threshold` (attribute `threshold`), `fallback-percentage` and
  `representative-claim` are ignored, and `overage-status` gives a window with a status but
  no utilization, which yields no meter. A window needs a parsable `utilization`; an
  unparsable `reset` leaves `resets_at: None`.
- Window names: `5h` → `FiveHour`, `7d` → `SevenDay`, anything else → `Named(label)` when it
  passes `Label::parse` (so `7d_oi` is shown as a named meter), otherwise dropped. The plan
  comes from the login probe (`planTier` of `claude auth status`: `max_5x`), not from the log.
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
The three lines are written at once, without waiting for the `initialize` response; that
the server accepts this (the spike sent them one by one) is **assumed** and is part of
T04's one real run; the fallback (the next request is written when a line is answered:
a `Flow` variant that carries the bytes to write) changes the frozen `proc` types, so it
is a contract change request to the lead, not a worker's edit.
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
Gate (`needs_login`): spawn only when the login probe (8.6) says Codex is `Ready`. Otherwise,
without spawning anything: `NotInstalled` → `Unavailable(NotInstalled)`, `NotLoggedIn` →
`Unavailable(NotLoggedIn)`, `Unknown` (the probe timed out or could not read the status) →
`Unavailable(Timeout)`, and the next attempt re-probes first (7.2). The Codex plan is the
response's `planType`; the probe supplies no plan for Codex.

### 6.4 DeepSeek balance
`GET https://api.deepseek.com/user/balance`, `Authorization: Bearer <key>`. Documented body:
`{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00",
"granted_balance":"10.00","topped_up_balance":"100.00"}]}`. One `Balance` meter per currency,
amounts parsed exactly by `Money::parse_decimal` (a second entry that maps to an id already
present, for example two unknown currencies, is dropped). No key → `NotConfigured(AddDeepSeekKey)`
with no network call. 401/403 → `Unauthorized`. Unverified until the user has a key.

### 6.5 Placeholders and the path to real adapters
`PlaceholderSource::new(provider)` returns `Unsupported`. Its descriptor is
`Trigger::OnSignal`, `Cost::Free`, `needs_login: false`: nothing ever signals it, so it is
asked once at start and when the window opens, never on a timer, and it does no IO. The
UI lists these under "No usage interface yet" (section 9.3). Later, without touching the service or the UI:
- Antigravity: an `AntigravityUsageSource` running `agy -p /usage --output-format json`
  through `proc`, once verified against a really installed binary.
- Grok: an `XaiPrepaidBalanceSource` on the Management API with `XAI_MANAGEMENT_KEY` and
  `XAI_TEAM_ID` in the env file, through `http` (a second allowed host).
- Gemini: none until Google ships a headless quota command.

## 7. Runtime architecture

### 7.1 The service loop
`jev-usage-io::service::UsageService` is one task that owns all mutable state. No lock
guards service state and no other task mutates it. Two things are shared on purpose and
written only by this task: the snapshot and settings `watch` channels, and `FactsCell`
(the login facts the sources read; the service **merges** a probe's answer into it for the
providers that were probed and leaves the others as they were). **Everything it touches is injected**,
so every behaviour is testable with fakes:
```rust
pub struct ServiceDeps {
    pub clock: Arc<dyn Clock>, pub timer: Arc<dyn WallTimer>, pub jitter: Arc<dyn Jitter>,
    pub sources: Vec<Arc<dyn UsageSource>>,
    pub probe: Arc<dyn LoginProbe>, pub facts: Arc<FactsCell>,       // CLI logins (io::probe, 8.6)
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
/// `rt` is the runtime the task runs on. The shell passes Tauri's (`tauri::async_runtime`),
/// because Tauri's `setup` hook is not inside a runtime and a bare `tokio::spawn` would panic.
pub fn spawn(deps: ServiceDeps, rt: &tokio::runtime::Handle) -> ServiceHandle;

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
- **Event-driven first**: `watch` uses inotify (the `inotify` crate on the runtime's
  reactor: no watcher thread) with **two narrow watches**: the log file itself for
  `MODIFY`, and its parent directory for `CREATE | MOVED_TO | MOVED_FROM | DELETE` only (so
  rotation by rename, deletion and re-creation are seen, and the file watch is moved to the
  new file). The gateway's other logs live in the same directory and are written far more
  often; with these masks their writes wake nothing. Directory events are filtered to the
  log's file name. **Throttle, not debounce**: the first event arms a 1.5 s timer and the
  signal is sent when it fires, so a continuous stream of writes signals every 1.5 s and can
  never starve the reader. No polling of the log, ever. An inotify queue overflow counts as
  a change. If the directory does not exist yet, or the watch is lost (directory removed),
  the watcher re-arms itself at 1, 2, 4 … 15 minute intervals until it succeeds, signals
  once when it does, and emits `DiagEvent::WatchLost`; no fetch loop is involved.
- **Signal sources** are also fetched once at start and once whenever the window becomes
  visible (a `stat`, in case a signal was lost).
- **Intervals** only for sources with no push channel (Codex, DeepSeek), longer while the
  window is hidden. A refresh also runs when the window becomes visible if the reading is
  older than the visible interval.
- **Backoff**: after the n-th consecutive failure (n ≥ 1) the next attempt is
  `min(60 min, 1 min x 2^(n-1))` (1, 2, 4 … 60 min) with ±10 %
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
- **Login probe** (`LoginProbe`; the real one, `CliProbe`, runs `claude auth status` and
  `codex login status` under the rules of 8.6, and only for providers that are not hidden):
  (a) once at start; (b) before a fetch of a `needs_login` source when the cached readiness
  of that provider is not `Ready` and the cached facts are older than 60 s, so a probe that
  timed out during a busy login is retried on the source's backoff schedule **even while
  the window is hidden**; (c) on a manual refresh, at most once per 60 s; (d) otherwise when
  the facts are older than 6 hours, **or older than 15 minutes while a probed provider's
  readiness is `Unknown`**, piggy-backed on a fetch that is due anyway (no timer of its
  own). The second case is for Claude: its only `needs_login` source is the manual live
  check, so (b) never retries a `claude auth status` that timed out during a busy login,
  and the plan name would otherwise stay missing for 6 hours. The probe never runs more
  often than once per 60 s.
- **Suspend/resume and clock jumps**: handled by the wall-clock timer (7.1). Every wake
  recomputes deadlines from the injected clock; deadlines found in the past run once each
  (no catch-up storm).
- The UI's countdown text is recomputed in the web view only while the window is visible
  (one timer aligned to the minute, stopped on `visibilitychange`). The backend never pushes
  ticks. Tray text uses absolute times, so it needs no ticking either.

### 7.3 Tauri command surface (complete)
All in `jev-dashboard::commands`. Arguments and results are the generated types of section
7.4. Errors are the closed enum `CommandError` inside `Outcome`; no command returns a string.

| command | args | result | notes |
|---|---|---|---|
| `get_snapshot` | none | `Outcome<Snapshot>` | read of the latest state; always `ok` |
| `refresh` | `target: RefreshTarget` | `Outcome<null>` | returns when queued (7.2); the result arrives as a `snapshot` event |
| `get_history` | `query: HistoryQuery` | `Outcome<HistorySeries>` | at most 600 points |
| `get_settings` | none | `Outcome<Settings>` | always `ok` |
| `update_settings` | `patch: SettingsPatch` | `Outcome<Settings>` | validated in core; the shell then applies the autostart entry (9.13) |
| `get_diagnostics` | none | `Outcome<Diagnostics>` | version, per-source state, store size |

**Every command resolves; none rejects on purpose.** The result is the generated envelope
`Outcome<T>` (7.4), so the UI receives a fully typed value on success *and* on failure and
never has to inspect a rejection, whose type in TypeScript would be `any` (1a). In Rust the
async commands are declared `Result<Outcome<T>, CommandError>` only because Tauri requires a
`Result` from an async command that borrows state; they always return `Ok`.

An argument that does not deserialise (an unknown provider, a meter id that
`MeterId::parse_key` rejects) is refused by Tauri before the command runs; that is the one
case in which the promise rejects (the UI's types make it unreachable). `ipc/tauri.ts`
attaches a rejection handler **with no parameter** that returns the constant
`{ status: "error", error: { code: "invalid_argument" } }`, so the rejection's untyped value
is never bound to a name. Values that deserialise but are out of range (thresholds) are
refused by core with the same code.

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
That `WebviewUrl::App` keeps a fragment is **assumed** (checked in T11); the fallback is an
initialisation script that sets one frozen global from the same closed vocabulary.

### 7.4 IPC types: exactly what crosses the boundary
Defined in `jev-usage-core::ipc`, `#[serde(rename_all = "snake_case")]`, internally tagged
enums whose tag is the one shown in the block below (`state` for `UsageView`, `kind` for
`MeterView`, `WindowKind`, `StaleCause` and `RefreshTarget`, `name` for `Route`, `status` for
`Outcome`, `code` for `CommandError`). TypeScript is generated by `ts-rs` into
`dashboard/src/lib/ipc/bindings/` and checked for drift in CI. `u64`/`i64` fields carry
`#[ts(type = "number")]` (all values are far below 2^53; `ts-rs` would print `bigint`), and
`MeterId`/`Label` fields carry `#[ts(type = "string")]` (their serde impls are hand-written).
The attribute replaces the **whole** field type, wrapper included (read in `ts-rs` 10's
source), so an `Option` field says `"number | null"` or `"string | null"` and a `Vec`
field `"number[]"`; a bare `"number"` on `Option<u64>` would type a value that is `null`
at run time as a number. A T00 test serialises a value with every optional field `None`
and checks each such key against the generated file's `| null`.
A few IPC types live beside their logic instead of in `ipc.rs` (`Settings` and
`SettingsPatch` in `settings`, `Severity` and `Phase` in `assess`); `ipc` re-exports them
and every rule for IPC types applies to them wherever they are defined (8.10).
No IPC type is a unit struct, an empty tuple struct or an empty enum: `ts-rs` 10 prints
those as `Record<string, never>`, `never[]` and `never` (read in its source), which 1a bans. The block below shows the shapes;
where `ts-rs` prints an equivalent type differently, the generated files are authoritative.
The generator is `ts-rs` (verified on a scratch crate with these shapes, version 10: tagged
enums, struct variants, `Option`, `Vec`, the generic `Outcome<T>`, `()` as `null`, optional
fields; the output contains no `any`, `unknown` or `never`). The type check of section 10
runs over the generated files too, so a future generator version cannot slip one in.
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
type RefreshView  = { in_flight: boolean; last_attempt_at: number | null; next_at: number | null;
                      free_refresh: boolean;                  // has a free source that a manual refresh starts
                      live_check: boolean;                    // has a spending source AND the setting is on
                      live_check_error: Unavailable | null }; // why the last live check failed; null after a success
type ProviderView = { provider: ProviderKey; enabled: boolean; usage: UsageView; refresh: RefreshView };
type Headline     = { provider: ProviderKey; meter_id: string; used_bp: number;
                      severity: Severity; resets_at: number | null };
type Snapshot     = { generated_at: number; providers: ProviderView[]; headline: Headline | null };

type RefreshTarget = { kind: "all" } | { kind: "provider"; provider: ProviderKey }
                   | { kind: "live_check"; provider: ProviderKey };   // the only one that may spend
type HistoryRange  = "day" | "week" | "month" | "quarter";
type HistoryQuery  = { provider: ProviderKey; meter_id: string; range: HistoryRange };
type SeriesUnit    = "basis_points" | "micros";
type HistoryPoint  = { t: number /* epoch s */; v: number; lo: number; hi: number };
type HistorySeries = { provider: ProviderKey; meter_id: string; from: number; to: number;
                       unit: SeriesUnit; points: HistoryPoint[];
                       gap_s: number;       // a hole longer than this is drawn as a gap (history::gap_for)
                       resets: number[] };  // window ends inside the range (epoch s), at most 64; may be empty (7.5)

type Theme     = "system" | "light" | "dark";
type HourCycle = "auto" | "h12" | "h24";
type Settings  = { version: number /* 1 */; warn_bp: number; critical_bp: number;
                   notifications: boolean;              // master switch (also the tray checkbox)
                   notify_warn: boolean; notify_critical: boolean;
                   theme: Theme; hour_cycle: HourCycle;
                   zoom_pct: number;                    // 100 | 125 | 150 | 175 | 200 (9.10)
                   close_to_tray: boolean; autostart: boolean; shortcuts: boolean;
                   claude_live_check: boolean; first_run_seen: boolean;
                   hidden_providers: ProviderKey[] };
type SettingsPatch = Partial<Omit<Settings, "version">>;
type CommandError  = { code: "invalid_argument" | "busy" | "not_enabled" | "store_unavailable" | "io" | "internal" };
type Outcome<T>    = { status: "ok"; value: T } | { status: "error"; error: CommandError };   // generated (generic), 7.3
type SourceId      = "claude_gateway_log" | "claude_live" | "codex_app_server" | "deepseek_balance"
                   | "gemini_none" | "antigravity_none" | "grok_none";
type SourceDiag    = { source: SourceId; provider: ProviderKey;
                       last_ok_at: number | null; last_error: Unavailable | null; failures: number };
type Diagnostics   = { app_version: string /* semver */; sources: SourceDiag[];
                       history_bytes: number; history_samples: number; tray_available: boolean };
type Route         = { name: "overview" } | { name: "provider"; provider: ProviderKey } | { name: "settings" };
```
**String-typed leaves that cross the boundary, exhaustively**: `plan` and `WindowKind.label`
(both `Label`: `[a-z0-9_]{1,40}`), `id`/`meter_id` (`MeterId`, serialised as its key: a fixed
prefix plus digits or a `Label`), `app_version` (`&'static str`, a compile-time constant).
Every other field is a number, a boolean or a closed enum. In Rust none of these is a
`String` field: they are `Label`, `MeterId` and one `&'static str`. There is no field that
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
  `auto_vacuum=INCREMENTAL`, `foreign_keys=ON`, `secure_delete=ON`, `busy_timeout=2000`,
  `temp_store=MEMORY` (a large `GROUP BY` never spills into a temporary file outside the
  state directory).
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
  with min, max and last), so a query never loads the raw rows of a range into memory. The
  store sees only `from` and `to`: a span of at most `RAW_RETENTION` (day, week, month) is
  served from `sample` alone; a longer span (quarter) is a **rollup query**: `rollup_hour`
  for hours before the oldest raw sample plus `sample` for the rest, without overlap. It returns the points, the bucket width, and the reset marks. Marks are also
  bounded in SQL: the `resets_at_s` of raw samples in range are grouped into buckets of half
  the window length (1 hour when the length is unknown) and at most 130 groups are read;
  `history::reset_marks` then collapses neighbours. `resets` is **empty** for a rollup
  query (the quarter range: rollups carry no reset time) and whenever more than 64 marks would result (a 5-hour
  window over a month): a partial set of marks would mislead.
- Rollups are written only by the prune: raw samples older than raw retention are folded
  into `rollup_hour` (`lo/hi/last` per hour) and deleted in the same transaction.
- `meta` also holds the shell's last window size (`MetaKey::WindowWidth`,
  `MetaKey::WindowHeight`): two integers, no new file and no new IPC field.
- Retention: raw samples 45 days (covers the 30-day Codex window and the month range);
  hourly rollups (`lo/hi/last`) 400 days; prune at start and every 24 h in one transaction, then
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
- A failure never erases a good reading: `Known` → `Stale{Failing(reason)}`. Freshness is
  the descriptor's `fresh_for`: gateway log and live check 10 min; interval sources 2.5 x
  their **hidden** interval (Codex 37.5 min, DeepSeek 75 min), one fixed value per source, so
  opening the window never makes a reading that was current a moment ago look stale.
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
| A lazy installer or self-updater runs (mise wrappers: `grok`, `agy`, and `codex`/`claude` themselves, 3.1) | 8.6: no variant for the first two; an install-on-disk check before any start; mise's installers switched off in the child environment; process-group kill. |
| Secrets inherited by a child process | Every child, the two login-status commands included, gets a scrubbed, allow-listed environment (8.6). The dashboard never calls the engine's unscrubbed probe (5.1). |
| Network exfiltration or tracking | One HTTP client, one allowed host, HTTPS only, no redirects, no proxy, no telemetry, no updater; the web view has `connect-src` limited to IPC. |

### 8.3 What is never read
- `gateway.jsonl`, `turns.jsonl`, `code-turns.jsonl`, `orch*.jsonl`, `history`, `sessions/`,
  `gateway.out`: **never opened**. An architecture test fails if any of these file names
  appears in dashboard source outside the deny-list test itself. The rule is really an
  allow-list: in the gateway's state directory the dashboard opens `usage.jsonl`,
  `usage.jsonl.1` and its own `dashboard/` subdirectory, and nothing else (`archive/`,
  `orch/`, `cooldowns.json`, `mcp-delegate-*.json` and whatever is added later included).
  Honest limit: the directory watch (7.2) delivers the **names** of top-level entries that
  are created, moved or deleted there; they are compared with the log's name and dropped,
  never stored or logged.
- In `usage.jsonl`: only `ts_ms` and `ratelimit` keys beginning
  `anthropic-ratelimit-unified-`. Honest limit: reading a file necessarily brings its bytes
  through a private read buffer and the JSON tokenizer. Those buffers are reused, never
  logged, never copied out, and zeroized when the tail reader is dropped. Nothing but the
  parsed numbers leaves `parse_line`.
- OAuth/token files and homes of any CLI (`~/.claude*`, `~/.codex`, `~/.gemini`, `~/.grok`):
  never opened, listed or stat-ed by the dashboard (architecture test), and the engine's
  probe, which does read some of them, is never called (5.1). The only path the dashboard
  inspects outside its own files, the log and the env file is mise's `installs/<tool>/`
  directory, to see that an install exists (8.6): directory names only.
- `claude auth status` output (JSON that includes the account e-mail, org id and org name)
  is read by the login probe into one zeroizing buffer of at most 64 KiB and parsed by
  `core::parse::claude_auth`, a serde struct with exactly three fields (`loggedIn`,
  `planTier`, `subscriptionType`); every other key is skipped without being materialised,
  and the buffer is zeroized when the probe returns. `LoginFacts` holds a `Readiness` and a
  `Label` per provider and nothing else, so the e-mail cannot be displayed, stored, logged
  or sent over IPC, which is what 1a requires. **Label positions, exhaustively** (the only
  places a free string is ever taken from): `planTier`/`subscriptionType` of
  `claude auth status`, `planType` of the Codex rate-limit response, and the window part of
  an `anthropic-ratelimit-unified-*` key.
- Codex `account/read` is never called.
- From `~/.jev-router.env`: `io::envfile` reads the file into a zeroizing buffer, hands it to
  the engine's pure rule (`engine_bridge::parse_env`) for one named key (`DEEPSEEK_API_KEY` at fetch time,
  `JEV_USAGE_LOG` once at start) and zeroizes the buffer. The engine's own file readers
  (`envfile::lookup`, `path_from_env`) are never called.

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
  `Box<dyn Error>`, no foreign error type (`std::io::Error`, `rusqlite::Error`,
  `reqwest::Error`, `zbus::Error`) and no `to_string()` of one in any `pub` signature:
  foreign errors are classified where they occur and dropped (file helpers return the
  field-less `io::fsutil::FsError`, not `std::io::Result`).
- **Panics**: unwinding (5); the panic hook prints only the source file's
  **base name** and the line, never the payload and never the directory (a dependency's
  location is an absolute path under `~/.cargo`, which contains the user name). No crash
  reporter, no upload. Core dumps disabled.
- **IPC**: section 7.4 is exhaustive.
- **Notifications and tray**: provider name, window name, percentage, reset time. Built only
  by `core::text` from typed inputs.
- **Clipboard**: written only by the Copy buttons, only with constant command strings from
  `copy.ts` (`codex login`, `claude auth login`, `jev-claude`, `DEEPSEEK_API_KEY=`; a test
  keeps the two login commands equal to `engine_bridge::login_command`); never read.
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
- It is read by `io::secrets::deepseek_key(KeySources) -> Option<Secret<String>>` (taken by
  value: a `Secret` cannot be cloned out of a borrow; the
  sources are injected, so tests never touch the real environment or home): process environment
  first, else the env file through `io::envfile` (a `Zeroizing<String>` parsed with the
  engine's pure rule through `engine_bridge::parse_env`; DRY: same parsing rules as the engine).
- The header value is marked sensitive (`HeaderValue::set_sensitive(true)`), so the HTTP
  stack will not print it.
- Honest limits: the allocator may have left copies in freed memory before zeroization of
  intermediate buffers we do not own (the engine's parser returns a `String` we move, not
  copy; `std::env::var` returns a copy we own and zeroize). The TLS stack
  holds the header bytes until the request is written. Swap is outside our control.

### 8.6 Subprocesses
One module, `io::proc`, is the only place `std::process`/`tokio::process` may be used
(architecture test). Rules:
- Program names come from a closed enum (`Program::Codex`, `Program::Claude`). There is no
  variant for `grok`, `agy` or `gemini`, so they cannot be run.
- **Resolution, without running anything** (`proc::resolve(Program) -> Resolved`): take the
  first executable file of that name on the app's `PATH`. If it is a mise launcher (its
  path has a `command-wrappers` or `shims` component, or it resolves through symbolic links
  to a file named `mise`), the tool counts as installed only when some version directory
  under `<mise data>/installs/<tool>/` holds the tool's **executable**: a regular file
  with an execute bit named `<tool>`, directly in the version directory or in its `bin/`
  (the two layouts seen, 3.1; `<mise data>` is `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`,
  else `~/.local/share/mise`). A directory that merely has entries (an interrupted
  download) does not count. No hit, or a launcher with no executable on disk, is
  `NotInstalled` and **no process is started**, so neither mise's first-run installer nor
  the account wrapper's explicit `mise install`/`mise use -g` fallback (3.1) can be reached
  through a missing tool. Erring towards `NotInstalled` on an unknown layout is the safe
  side: the card says the tool was not found.
- The program is then started **by the name `PATH` gave**, wrapper included, because on this
  machine the wrapper is what selects the active account (3.1). The four-hourly self-update
  and any other install are switched off for the child with the constants
  `MISE_OFFLINE=1`, `MISE_AUTO_INSTALL=false`, `MISE_EXEC_AUTO_INSTALL=false` and
  `MISE_NOT_FOUND_AUTO_INSTALL=false` (these are mise's real variable names and it maps
  them to its settings: verified, 3.1; its documentation says offline blocks every HTTP
  request and that no update runs offline; that a wrapper run on this machine then never
  downloads and never rewrites mise's configuration is still **assumed** and checked in T04
  by comparing mise's `installs/`, `downloads/` and `incomplete-installs/` listings and a
  hash of `~/.config/mise/config.toml` before and after a real status run). The
  process-group kill below is the backstop for a download; nothing in the app can undo a
  configuration write, which is why the executable check above comes first. `codex` and
  `claude` are tools Omarchy declares, so the wrapper's branch for them is `mise install`
  (blocked by offline), not `mise use -g`. Honest limits: the check proves that *a* version
  is on disk, not that it is the version mise's configuration selects; when those differ
  the wrapper's `mise install` is reached and fails offline without a download (T04 shows
  this case with a stub, and the real run compares the listings);
  the switches are inherited by the real tool and anything it starts through mise; and a
  CLI's **own** updater (not mise's) is outside the app's control: the same before/after
  listing shows whether a status run triggers one.
- Arguments are constant arrays. No shell, no string interpolation.
- Environment: `env_clear()` then exactly these, each only if set in the parent: `HOME`,
  `USER`, `LOGNAME`, `PATH`, `LANG`, `LC_ALL`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
  `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS` (a CLI
  that keeps its login in the session keyring needs the bus), `CODEX_HOME`,
  `CLAUDE_CONFIG_DIR`, `MISE_DATA_DIR`, `MISE_CONFIG_DIR` (so the child finds the same login
  and the same installed tool the user's shell does; all four are directory paths, not
  secrets); plus the constants `TERM=dumb`, `DISABLE_AUTOUPDATER=1` (Claude Code's switch for
  its **own** updater; that this is its name and that it is honoured is **assumed**, checked
  by T04's before/after listing) and the four `MISE_*` switches above. No
  `*_API_KEY`, `*_TOKEN` or proxy variable ever reaches a child. For `ClaudeLive`,
  `ANTHROPIC_BASE_URL` is not set, so it does not go through the gateway.
- Working directory: `<state_dir>/dashboard/run` (empty, 0700), so no project config is read.
- stdin/stdout piped. **stderr is discarded**, with one exception: the two login-status
  specs merge it into stdout (`Stderr::Merge`), because `codex login status` answers on
  stderr (3.1). Output caps: 1 MiB per line, 4 MiB total (64 KiB total for the status
  commands). The child is the leader of its own process group. The group is killed
  (`SIGKILL`) on the deadline **and whenever the run is dropped** (service timeout,
  shutdown), by a guard that is disarmed once the child has been reaped; `kill_on_drop`
  alone would leave the real tool running behind its launcher (`codex` and `claude` are
  wrappers here). If the app itself is killed, a `codex app-server` child ends on its own
  when its stdin closes (3.1).
- Child output is parsed by an allow-list parser and never logged or stored raw. Every
  buffer `proc` reads child output into is zeroized when the run ends (the status output
  contains the e-mail, 3.1), not only the probe's own buffer.
- **The login probe** (`io::probe::CliProbe`, the only `LoginProbe` used in production),
  for Codex and Claude only, and only for a provider that is not hidden:
  1. `proc::resolve`; `NotInstalled` ends here with nothing run.
  2. Run `codex login status` or `claude auth status` through `proc` under every rule
     above, deadline 5 s, output collected into one zeroizing buffer.
  3. Codex: `engine_bridge::codex_logged_in(exit_ok, text)` (the engine's pure rule; non-UTF-8
     output is `Unknown`). Claude: `core::parse::claude_auth::parse(&[u8])`, the three-field
     allow-list of 8.3, with the engine's text fallback re-stated on bytes (contains
     "not logged in" or "not authenticated" → logged out; else contains "logged in" →
     logged in; else unknown). The plan is `planTier`, else `subscriptionType`,
     lower-cased, `-` and space turned into `_`, through `Label::parse`.
  4. `true` → `Ready`, `false` → `NotLoggedIn`, unreadable or timed out → `Unknown`.
  It opens no file of any CLI and never looks at DeepSeek, Gemini or Grok.

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
  second process exits before it opens the store, the log or a child. The plugin hands the
  first instance the second one's arguments **and working directory** (a path); the shell
  tests the arguments for `--hidden` and drops both, never storing or logging them. The plugin's lock is
  a name on the session bus derived from the identifier, so a debug build or a test run
  would be swallowed by the user's resident Flashboard (autostart is on by default).
  Therefore: `pnpm dev` passes `--config` with the identifier `dev.jev.flashboard.dev`, debug
  builds keep their files in `<state_dir>/dashboard-dev`, and every automated launch of the
  real binary (11.4 stages 10 and 13, `measure-idle.sh`) runs under `dbus-run-session`,
  which gives it a private bus (no clash). A private bus alone still **activates** services
  from the system's service files (a notification daemon, portals), which would then draw on
  the user's desktop with the test's environment; so the launches pass
  `--config-file dashboard/e2e/smoke/session-bus.conf`, a bus configuration with no service
  directory, on which nothing can be activated: notifications fail into
  `DiagEvent::NotifyFailed` and no tray host exists. That WebKitGTK and GTK start without
  a portal or accessibility service on that bus is **assumed** (first run of stage 10; the
  fallback is a service directory holding only the services that prove necessary, never a
  notification daemon). **Not** compiled in:
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
  `app.security.freezePrototype: true` (that Svelte 5's runtime works with a frozen
  `Object.prototype` is **assumed**; T00 checks it on the scaffold window and reports);
  `app.security.dangerousDisableAssetCspModification: false`; no `assetProtocol`; no remote
  URLs in `capabilities`; CSP:
  ```
  default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self';
  font-src 'self'; connect-src ipc: http://ipc.localhost; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'; object-src 'none'
  ```
  No `'unsafe-inline'`, no `'unsafe-eval'`, no `data:`. This is the policy of every built
  binary. `pnpm dev` loads the UI from Vite's dev server, whose hot reload needs a
  WebSocket to localhost and injected `<style>` elements; whether Tauri applies `csp` to
  that page is **assumed** not (T00 checks). If it does, T00 sets `app.security.devCsp`,
  which only development runs read, to this policy plus exactly those two allowances; the
  manifest snapshot test pins `csp` itself. Consequences for the UI, all
  enforced by lint or test: Vite's `build.assetsInlineLimit` is `0` (no asset is ever inlined
  as a `data:` URI, which this CSP would block); icons are inline SVG in the markup; dynamic
  styles are set through Svelte's `style:` directive (the CSSOM), never through a `style="…"`
  attribute or an injected `<style>`, both of which this CSP blocks; animations are CSS.
- The window is built with `incognito(true)`: WebKit keeps cookies, storage and caches in
  memory only and leaves nothing under `~/.local/share` or `~/.cache`.
- Navigation is locked: `on_navigation` allows only the app origin; new windows are denied.
- The window is built with `disable_drag_drop_handler()`. Tauri's default handler turns a
  file dropped on the window into a `tauri://drag-drop` event whose payload is the file's
  **path**, and the capability's `allow-listen` would let the page receive it: the one
  channel by which a path could reach the web view. With the handler off no such event
  exists (the UI has nothing to drop onto).
- Devtools: the `devtools` Cargo feature is not enabled; a test asserts the release manifest.
- The web view's context menu (which offers Reload and Inspect) is suppressed by the UI in
  production builds (a `contextmenu` listener in `main.ts`; Tauri has no Rust switch for it
  on Linux). A reload is harmless anyway: the UI rebuilds itself from `get_snapshot`.
- Exit: `RunEvent::ExitRequested` is prevented only when it has no exit code (the last window
  closed while the tray is alive); an explicit `app.exit(0)` from the tray's Quit goes
  through, after `ServiceHandle::shutdown()`. `SIGTERM`, `SIGINT` and `SIGHUP` (logout, the
  session unit stopping) take the same path through a tokio signal listener: shutdown, then
  `app.exit(0)`; without it a signal would leave a child's process group behind.
- Zoom (9.10) is applied from Rust with `WebviewWindow::set_zoom` from `settings.zoom_pct`,
  at window creation and on change. Tauri's own zoom hot keys stay off: on Linux they are a
  script polyfill that needs a webview permission, which this capability does not grant.
- At start (`hardening::apply`), before any thread or child exists: `umask(0o077)`, core
  dumps off (`RLIMIT_CORE = 0`), `PR_SET_DUMPABLE=0`, the panic hook. Uses safe wrappers from
  `rustix`; `unsafe` is forbidden (`#![forbid(unsafe_code)]`) in all three app crates (1a).
  That Tauri's `generate_context!`, `generate_handler!` and `#[tauri::command]` expansions
  compile under it **and** pass the `disallowed_types` lint (they handle JSON values
  internally), and that the `rustix` calls exist as safe functions (`umask`, `setrlimit`,
  `set_dumpable_behavior`, `kill_process_group`, the timerfd calls: present in rustix 1.1,
  read in its source), is **assumed** and confirmed by T00 on the scaffold; if one does not,
  the lead brings it to the user instead of relaxing the rule. `RLIMIT_CORE = 0` is
  inherited by every child, WebKit's helpers included; the dumpable flag is not (it resets
  on `exec`). Children inherit
  the umask (their files become private, which is harmless). `PR_SET_DUMPABLE=0` makes
  `/proc/<pid>` owned by root; if the T11 manual check finds that this breaks a portal
  lookup, WebKit's sandbox helper or its accessibility bridge on this machine, it is dropped
  and `RLIMIT_CORE = 0` stays (risk in section 14). Honest limit of that fallback: with a
  piped `core_pattern` (systemd-coredump, as here) the kernel still calls the handler, which
  honours the zero limit by storing no memory image but does log the crash with a stack
  trace of function names.

### 8.9 Dependency and supply-chain policy
- Rust: lockfile committed, every command `--locked`. `deny.toml`: advisories = deny
  (vulnerabilities, unsound, yanked), sources = crates.io only (no git), licenses = explicit
  allow-list, bans = `openssl`, `openssl-sys`, `native-tls` denied, wildcard versions denied.
  `cargo audit` as a second advisory source.
- Direct Rust dependencies, complete list (additions need lead approval):
  core: `serde`, `serde_json`, `zeroize`, `ts-rs` (dev/bindings feature); dev: `proptest`,
  `static_assertions`, `syn` (feature `full`, for the type-file test). io: `jev-usage-core`,
  `tokio`, `inotify` (default features off: its descriptor is driven by tokio's `AsyncFd`
  like the timerfd, so no stream adapter crate is needed), `rusqlite` (bundled), `reqwest`
  (the engine's line: no default features, `rustls-tls`; never `native-tls`),
  `rustix`, `zeroize`, `serde_json`, and the engine facade as `jev-engine` (the one line of
  5.1); dev: `tempfile`, `proptest`. shell:
  `jev-usage-core`, `jev-usage-io`, `tokio` (signals, the 30 s timer, channel types),
  `tauri` (feature `tray-icon` added to the defaults; dev: feature `test`), `tauri-build`,
  `tauri-plugin-single-instance`, `notify-rust` (default features off, `z` only: the zbus
  backend, no image support), `zbus` (the version `notify-rust` already resolves; T00
  reports if the single-instance plugin pulls a second major version), `chrono` (no default
  features, `clock`), `rustix`; dev: `tempfile`, `serde_json`. There is no `rand`:
  `SystemJitter` takes its bits from `std::collections::hash_map::RandomState`.
- JavaScript: `pnpm-lock.yaml` committed, `--frozen-lockfile`; exact versions
  (`save-exact=true`); `.npmrc`/workspace settings: dependency build scripts blocked except an
  explicit allow-list (`esbuild`), `minimumReleaseAge` of 7 days for new versions;
  `pnpm audit --prod` must be clean. Runtime dependencies: `svelte`, `@tauri-apps/api`, two
  font packages (assets only). Everything else is a dev dependency: `vite`,
  `@sveltejs/vite-plugin-svelte`, `svelte-check`, `typescript`, `vitest`,
  `@vitest/coverage-v8`, `jsdom`, `@testing-library/svelte`, `@testing-library/user-event`,
  `axe-core`, `@axe-core/playwright`, `@playwright/test`, `@types/node` (the config files,
  `check-types.ts` and the e2e specs use Node's API), `eslint`, `typescript-eslint`,
  `eslint-plugin-svelte`, `jiti` (ESLint needs it to load a TypeScript config), `stylelint`,
  `stylelint-declaration-strict-value` and `postcss-html` (the token rule inside `.svelte`
  files), `prettier`, `prettier-plugin-svelte`, `@tauri-apps/cli`. T00 may swap one of these
  for an equivalent only with lead approval and reports the final list; a tool missing from
  this list is a contract change, not a worker's choice.
- No CDN, no remote font, no remote script: everything is bundled and served from the app.
- Build tools fetched at install time (`@tauri-apps/cli`) are pinned in the lockfile.

### 8.10 Tests that prove the rules
| rule | proof |
|---|---|
| No prompt can surface from the log parser | **Canary fixture**: `dashboard/fixtures/claude-gateway/canary.jsonl` has the canary string `JEVCANARY_9f3c1b7e_DO_NOT_SURFACE` in every non-allow-listed key, as extra keys, inside allow-listed keys' values, and as a window name. Test: `format!("{:?}")` and the JSON of everything `parse_line` returns contain no `JEVCANARY`. Property test: for arbitrary JSON rows with the canary at arbitrary depth, the same holds. Because that canary is upper-case and `Label` would reject it anyway, the fixture and the property test also carry a **Label-shaped canary** (`jevcanary_9f3c1b7e`) in every position that is not a label position of 8.3: it must never surface, which proves the allow-list of positions and not just the character set. One documented test shows the converse (the same string as a window name does surface, by design). |
| No secret or identity crosses any adapter | Every adapter test feeds canary-laced input (Codex `account_read.response.json` with `canary-email@example.invalid`, a synthetic `claude auth status` output with the canary e-mail, org id and org name, a DeepSeek response with the canary as an extra field, the canary as the DeepSeek key) and asserts the canary is absent from the returned `Reading`/error Debug output. |
| End to end: nothing leaks to IPC, store, settings, logs | `io` integration test: a service wired with canary-fed sources and a temp state dir runs a full cycle; then every emitted `Snapshot` (JSON), every file under the temp dir (raw bytes, including SQLite WAL), and captured `diag` output are searched for `JEVCANARY`, the canary e-mail and the canary key. The real-binary smoke test (11.4) repeats the search over its whole temp tree, including `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_CONFIG_HOME` and `XDG_RUNTIME_DIR`, and over the binary's stderr. |
| IPC carries only allowed fields | Test serialises generated values (proptest strategies for every IPC type) and walks the JSON: every key must be in `ipc-allowlist.txt`; no key matches `(?i)email|token|secret|password|prompt|message|content|account|session|cwd|path|url|auth|key$`; every string leaf must match a closed-vocabulary set or `^[a-z0-9_:.+-]{1,48}$`. The same walk runs over the generated `.ts` bindings' property names. |
| `Secret` cannot be printed/cloned/serialised | `static_assertions::assert_not_impl_any!(Secret<String>: Debug, Display, Clone, Serialize, PartialEq)`. |
| One exit for secrets, one HTTP client, one process spawner, one D-Bus user | Architecture test (plain Rust test that scans `dashboard/**/*.rs`, tests excluded where noted): `expose_secret(` only in `io/src/http.rs`; `reqwest` only in `io/src/http.rs`; `process::Command` only in `io/src/proc.rs`; `zbus` only in `src-tauri/src/bus.rs`; `notify_rust` only in `src-tauri/src/notifier.rs`; the engine's crate identifiers only in `io/src/engine_bridge.rs` (5.1); forbidden file names (8.3) nowhere; no `println!`/`eprintln!`/`dbg!`; no `serde_json::Value`, `json!` or `unwrap`/`expect` outside tests, and `#![forbid(unsafe_code)]` at the top of all three crate roots (1a); the **type-file rule**: a `syn`-based test walks every struct field and enum variant field in the core type files (`time quantity label provider meter usage port secret ipc`) **and in every other core type that derives `TS`** (the IPC types defined beside their logic, 7.4) and fails on any field whose type mentions `String`, `str`, `Cow`, `OsString`, `PathBuf`, `Vec<u8>` or `Value`, with exactly two exceptions: `Label`'s own `Box<str>` and `Diagnostics.app_version: &'static str`. (Function signatures such as `MeterId::key() -> String` are not fields and are allowed.) `text.rs` builds display strings from typed inputs only and is not a type file. |
| Children the dashboard spawns get no secrets | Test spawns a stub program through `proc` with `DEEPSEEK_API_KEY=JEVCANARY...` in the parent environment; the stub prints its environment; assert no canary and only allow-listed names plus the constants of 8.6. The login probe's two status commands go through the same `proc` and are covered by the same test. |
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
  focus (WCAG 1.4.13), and never covers the control that opened it. `Esc` has one order of
  precedence everywhere: it closes an open dialog, else an open level-1 disclosure, else it
  goes back one view; one press does one of these. Its complete content is
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
One card per configured provider, ordered by severity (critical, warn, ok, unknown) then by
fixed provider order; ties never reorder on their own. A reorder never takes the keyboard
focus away: when a card moves while an element inside it has focus, that same element has
focus afterwards (moving a DOM node drops focus in WebKit, so the screen restores it), and
the level change that caused the move is what the live region announces (9.10). Providers without a usage interface are one
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
  inside one another. A meter row is an element with `role="meter"`, an accessible name
  ("Claude 5-hour") and `tabindex="0"`; Svelte's `a11y_no_noninteractive_tabindex` warning is
  suppressed on that one element with a written reason (the lead-approved exception: the
  focus is what gives keyboard users level 1). `aria-valuenow` is clamped to 0..100 while
  the text carries the real figure. A row with no number (elapsed, no data) is not a meter:
  it is plain text in the same tab stop ("5-hour: no data since the reset at 14:00").
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
- The top row is the one shared header (back, the provider's name, freshness, refresh,
  settings: the same component on every view). The plan name is the first element of the
  screen's own content, on the first meter's heading line; the wireframe draws it on the
  header row only to save a line, and omits the settings icon for the same reason.
- Chart: a **step** line (usage is piecewise constant between samples) with a soft area
  fill, vertical hairlines at window resets, dashed threshold lines labelled in text, gaps
  drawn as gaps (never interpolated across missing data). Rollup ranges draw the `lo..hi`
  band. Hand-written SVG, no chart library.
- Chart accessibility: the plot is `role="img"` with a generated summary ("5-hour usage over
  the last day: peak 72 % at 09:40, now 50 %, 3 resets") and a visually hidden data table of
  the points. Keyboard: a separate crosshair control with `role="slider"` (named "Inspect
  history", `aria-valuemin/max/now` = point index, `aria-valuetext` = "09:40, 72 percent")
  moves between points with Left/Right, Home and End; a screen reader speaks a focused
  slider's value by itself, so the readout does **not** go through the app's live region
  (whose 2 s merging would garble fast key presses). Pointer: hover shows the same readout;
  nothing needs a drag.
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
│   Size    (•) 100%  ( ) 125%  ( ) 150%  ( ) 175%  ( ) 200%   │
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
its previous state and says why (`CommandError` `io`). Under the shortcuts switch the
Behaviour section lists every key in text (9.10: this is what `?` opens; the wireframe
shows the short form). "Size" is the zoom of 9.10
(`zoom_pct`; also Ctrl + plus, minus and zero). "About and diagnostics" is the About
screen of 1a: the name Flashboard, the version, and the typed diagnostics.

### 9.7 Design system (tokens)
Tokens are defined once in `src/lib/design/tokens.css` as CSS custom properties. Components
use tokens only (a stylelint rule bans raw colours, raw font sizes and raw spacing in
component styles).

**Type.** Geist Variable for text, Geist Mono Variable for every number, time and command
(both OFL, self-hosted, latin subset, `font-display: block` since they are local). Fallback
`system-ui`, `ui-monospace`. All numerals `font-variant-numeric: tabular-nums`. Sizes are
authored in `rem` (the table gives them in px at the 16 px default), so a larger default
font size scales all text.

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
| `warn` | `oklch(54% 0.11 70)` | `oklch(80% 0.13 80)` | warn level (amber) |
| `critical` | `oklch(52% 0.19 30)` | `oklch(72% 0.17 32)` | critical level (vermilion) |
| `unknown` | `oklch(58% 0.006 250)` | `oklch(62% 0.006 250)` | no data (glyph and outline only, never a text colour) |

Computed for this review from the table (OKLCH → sRGB → WCAG contrast): every text colour
(`text`, `text-2`, `accent`, `warn`, `critical`) is ≥ 4.5:1 on `bg`, `surface` **and**
`surface-2` in both themes (lowest: light `warn` 4.69 on `surface-2`), `border-strong` and
`unknown` are ≥ 3:1, and every value is inside the sRGB gamut. The earlier light `warn`
(`56% 0.13 70`) was outside the gamut and 4.32:1 on `surface-2`; it is replaced above.
`border` (about 1.3:1) is decoration only: nothing that must be seen to operate or
understand the UI relies on it. Status words are set in `text`/`text-2` or their level's
hue, never in `unknown`.

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
`data-theme` on `<html>` before the app renders anything: the shell puts the saved theme in
the URL fragment when it creates the window (7.3), and `boot.ts`, the **first import of the
entry module**, sets the attribute. It cannot be an inline script (the CSP forbids that) or
a classic blocking script (Vite bundles TypeScript only as modules, and a hand-written
`.js` file is banned by 1a). A module runs before `DOMContentLoaded`, and `<body>` is empty
until the app mounts, so the only thing that could be painted earlier is the page
background. That WebKitGTK paints nothing before the module runs is **assumed** (T11's
manual list: a dark override on a light desktop shows no light flash); the fallback is the
shell giving the window the theme's `bg` as its native background colour. `color-scheme` is set so native
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
- **Money**: `Intl.NumberFormat` currency style, two decimals; `Currency::Other` (no code is
  kept) as a plain two-decimal number with the word "balance".
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
  control boundaries, focus ring, bar fill against track, status glyphs, and the chart's
  line and threshold lines against the plot background ≥ 3:1, in both themes, on every
  surface a colour is used on (`bg`, `surface`, `surface-2`); computed from the token file
  by a test.
- **Not colour alone** (1.4.1): every level has a glyph shape, a bar pattern and a word (9.7).
- **Keyboard** (2.1.1, 2.1.2): every function, no trap. Tab order equals reading order
  (2.4.3). Shortcuts: `R` refresh, `,` settings, `Esc` back, `1`–`6` open provider, `?`
  shortcut list. `1`–`6` follow the fixed order of `ProviderKey::ALL` (Claude, Codex,
  DeepSeek, Gemini, Antigravity, Grok), never the cards' current order, which changes with
  severity; the digit of a hidden provider does nothing. The shortcut list is not a
  dialog: `?` opens Settings, whose Behaviour section lists every key in text (the
  single-key ones, `Esc`, and the three zoom keys), so the list exists once and is
  reachable without knowing a shortcut. Single-key shortcuts act only when no text field
  has focus and can be switched off (`settings.shortcuts`, WCAG 2.1.4). `Esc` and Tab
  always work.
- **Focus** (2.4.7, 2.4.11, 2.4.13): a 2 px `accent` ring with 2 px offset on
  `:focus-visible`, never removed, never hidden behind the sticky header (scroll padding);
  focus moves to the view title on navigation and returns to the triggering control on back;
  dialogs trap focus and return it.
- **Names and roles** (1.3.1, 4.1.2): one `<main>`, a heading per view and per card; each
  meter is `role="meter"` with `aria-valuenow/min/max` and an `aria-valuetext` that carries
  the full level-1 detail ("50 percent used, resets in 2 hours 14 minutes, today at 14:00,
  status allowed, seen at 12:33"); icon buttons have text names; the chart is described
  (9.4).
- **Label in name** (2.5.3): where a control shows text, its accessible name contains
  that text, word for word and in the same order. The card header button shows the
  provider and the plan ("Claude", "Max 5x"), so its name begins "Claude, Max 5x" and the
  summary follows, never a rephrasing; a meter row shows "5-hour" and is named
  "Claude 5-hour". A test compares each control's visible text with its computed name.
- **Status messages** (4.1.3) that are **not spammy**: one polite live region for the whole
  app. It speaks only when (a) a meter changes level (ok, warn, critical), (b) a provider's
  state changes (for example to "can't reach Codex"), or (c) the user's own action finishes
  ("Updated 12:33", "Copied", "Saved"). A number that changes without changing level is
  never announced. Changes within 2 s are merged into one sentence, and the same sentence is
  not repeated within 60 s. Level changes use the wording of the notification (7.6), once.
  `role="alert"` is used only for an error that needs the user's action, once.
- **Pointer** (2.5.7, 2.5.8): targets ≥ 32 x 32 px; nothing needs a drag (sliders have a
  number field and arrow keys; the chart crosshair is a slider with arrow keys, 9.4).
- **Reflow and zoom** (1.4.4, 1.4.10, 1.4.12): a desktop web view has no browser zoom of its
  own, so the app provides it: `settings.zoom_pct` (100 to 200 % in steps of 25; Settings,
  and Ctrl + plus / minus / zero in the UI, which only call `update_settings`), applied by
  the shell to the whole web view (8.8) and remembered. The requirement, stated in CSS
  pixels so it can be tested: no horizontal scroll and no clipped or overlapping text at a
  viewport of **320 CSS px** wide, which is a 320 px window at 100 % and a 640 px window at
  200 % (the two cases the tests run; Playwright emulates the second as a 320 CSS px
  viewport at device pixel ratio 2). Below 320 CSS px (a 320 px window at 200 %) the page
  may scroll horizontally but nothing is clipped. Sizes are in `rem`; the four text-spacing
  values of 1.4.12 (line height 1.5, paragraph 2 em, letter 0.12 em, word 0.16 em) applied
  by a test lose no content.
- **Forms** (3.3.1, 3.3.2, 3.3.3, 1.3.1): every control has a visible label that is its
  accessible name; a threshold error is text next to the field, tied to it with
  `aria-describedby` and `aria-invalid`, and says what is allowed ("Warn must be below
  critical (95)"); nothing is reported by colour or by an automatic correction alone.
- **Updating content** (2.2.2): numbers update in place because that is the app's
  function; nothing moves, scrolls or blinks, and no message disappears on a timer except
  the "Copied" and "Saved" confirmations, which are also announced.
- **Page** (2.4.2, 3.1.1): `<html lang="en">`; the document title names the view
  ("Flashboard: Overview", "Flashboard: Codex", "Flashboard: Settings").
- **Motion** (2.3.3): everything in 9.7 stops under `prefers-reduced-motion`; nothing
  flashes; nothing loops.
- **Forced colours**: usable under `forced-colors: active`.
- **Disclosure** (1.4.13): the level-1 detail follows the rules in 9.1.
- **Focus kept** (2.4.3, 2.1.1): a live update never moves or drops the keyboard focus,
  including when cards reorder (9.3).

Automated, in CI: axe (`axe-core`) with **zero violations of any impact** on every component
test and on every screen x state x theme in Playwright; the contrast test; keyboard-path
tests (Tab order per screen, every shortcut, focus return); live-region tests (a level
change is announced once; twenty number updates announce nothing). Manual, in the definition
of done (11.5): a keyboard-only walk-through and a screen-reader pass with Orca against a
written script that lists what must be heard on each screen; the user takes part in that
pass (1a). Whether WebKitGTK exposes `role="meter"` with `aria-valuetext` to Orca is
**assumed**; the pass checks it first, and the fallback is the same text in a visually
hidden element referenced by `aria-describedby`.

### 9.11 Tray
- Icon: a ring gauge drawn at run time (pure function, RGBA, 64 px, cached per 5 %-step and
  level). Fill arc = the headline usage. Level is also a shape: ok = ring only; warn = ring
  plus a centre dot; critical = ring plus a filled centre; no data = dashed ring. A mid-grey
  outline keeps it legible on light and dark panels.
- Linux has no tray tooltip (the StatusNotifier library Tauri uses exposes none), so the
  "tray tooltip" of 1a is the item's title: the tray item's id and title are "Flashboard".
  **Assumed**: that this host shows the title as a name (in its own tooltip or list) and
  not as a text label painted beside the icon, which is what the library's Linux `set_title`
  maps to in some hosts. T11's manual list checks it; if a label appears in the panel, the
  title is left unset and the id alone names the item.
- Menu (Linux tray hosts deliver the menu, not click events, so everything is in the menu):
  ```
  Claude · 5-hour 50% · resets 14:00          (a row opens that provider's detail view)
  Claude · 7-day 37% · resets Thu 02:00
  Codex · 30-day 4% · resets 16 Dec
  Codex · 30-day 4% · as of 16 Dec 12:31      (a stale reading says so instead of the reset)
  Claude · 5-hour · no data since reset       (an elapsed window shows no number)
  Codex · not signed in                       (no reading: one row from a closed phrase table)
  ─────────────────────────────
  Open dashboard
  Refresh now
  [x] Notifications
  ─────────────────────────────
  Quit
  ```
  Rows use absolute reset times in a form that stays true without a timer: a reset within
  the next 24 hours is "HH:MM", within 6 days "Thu HH:MM", later "16 Dec" (no "today" or
  "tomorrow", which would go wrong at midnight). A stale row's "as of" is always date and
  time, which stays true however long the row is not rebuilt. A provider without a reading
  has one row whose phrase comes from a closed table in `core::text` keyed by `Unavailable`
  and `Setup` ("checking" while pending); hidden and unsupported providers have no row.
  Rows are enabled items (disabled items are
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
  stale; when the window has no reset time the body is omitted). Application name "Flashboard", `desktop-entry` hint `dev.jev.flashboard`. Warn
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
  file does not exist. Pure text generation, atomic write, 0600. The binary's path is taken
  **once, at start** (`current_exe`; later it could read "… (deleted)" after a reinstall),
  must be valid UTF-8 and an existing file, and is quoted for `Exec=` as the Desktop Entry
  specification requires (double quotes; `"`, `` ` ``, `$` and `\` escaped; `%` doubled).
  If the entry cannot be written at start, the shell sets `autostart` to off through the
  service (so the switch tells the truth) and emits `DiagEvent::AutostartFailed`. Debug
  builds never write it. The first run tells the user and offers the switch (9.5). This session honours XDG
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
| Framework | **Svelte 5** (runes) + TypeScript at the strictness of 1a (every component `lang="ts"`, typed `$props()`) | Compiles to small direct DOM code, about a third of a React bundle here; fine-grained updates suit a few live numbers; scoped styles; no runtime CSP issues. |
| Build | Vite, static output (no SSR, no router package), `build.assetsInlineLimit: 0`. Svelte's options are passed inline to the plugin in `vite.config.ts` with `configFile: false`; there is **no `svelte.config.js`** (1a bans `.js`). That `svelte-check` and the ESLint plugin need no config file is **assumed**; T00 checks it, and reports to the lead if one of them insists on the file | Tauri serves static files; nothing is inlined as `data:` (8.8). |
| State | One `usage` store (a rune-based class) fed by `get_snapshot` + the `snapshot` event; `settings` store; a tiny in-memory router. Files that use runes outside a component are named `*.svelte.ts` (Svelte 5 compiles runes only there) | The backend owns the state; the UI holds a read model. No state library. |
| IPC | `@tauri-apps/api` `invoke`/`listen`, wrapped once in `src/lib/ipc/client.ts` behind an `IpcClient` interface (every method returns `Promise<Outcome<T>>`) with a mock implementation for tests | One seam; components never import Tauri and never see a rejection. |
| Chart | Hand-written SVG component (`Chart.svelte`, about 250 lines) | Full control of accessibility, theming, step rendering and gaps; no dependency. |
| Styles | Plain CSS with custom properties, scoped per component | No CSS framework, no runtime. |
| Icons | Inline SVG sprites (about 12 icons, own set) | No icon package. |
| Fonts | `@fontsource-variable/geist` and `geist-mono`, latin subset | Local, consistent numerals. |

**Type discipline (1a), enforced three times.**
1. *Compiler*: `tsconfig.json` has `strict`, `noUncheckedIndexedAccess`,
   `exactOptionalPropertyTypes`, `noImplicitOverride`, `noPropertyAccessFromIndexSignature`,
   `noImplicitReturns`, `noFallthroughCasesInSwitch`, `noUnusedLocals`, `noUnusedParameters`,
   `allowUnreachableCode: false`, `verbatimModuleSyntax`, `allowJs: false`,
   `erasableSyntaxOnly` (so Node runs `check-types.ts` directly, no loader dependency) and
   `resolveJsonModule` (tests import `format-vectors.json` as a typed module; that is the
   only way JSON enters TypeScript).
   `useUnknownInCatchVariables` stays on as part of `strict`, and can never matter because
   catch bindings are banned (below).
2. *Lint* (`typescript-eslint` `strictTypeChecked` and `stylisticTypeChecked`, with type
   information, over `.ts`, `.svelte.ts` and `.svelte`; run with `--no-inline-config` and
   `--max-warnings 0`, so no rule can be switched off in a file): `no-explicit-any`,
   the `no-unsafe-*` family, `consistent-type-assertions` set to `never`,
   `no-non-null-assertion`, `ban-ts-comment` (all directives, no descriptions accepted),
   `no-restricted-types` for `Function`, `Object`, `object` and `{}`, `switch-exhaustiveness-check`,
   `explicit-module-boundary-types`, and `no-restricted-syntax` for the AST nodes
   `TSAnyKeyword`, `TSUnknownKeyword`, `TSNeverKeyword`, `TSTypeAssertion`, `TSAsExpression`
   (except `as const`), `TSNonNullExpression` and `CatchClause[param]`, plus the
   **assertions in disguise**: `TSTypePredicate` (`x is T` and `asserts x`: an unchecked
   claim), every `declare` of a value, function, module or global (the one ambient file
   `src/env.d.ts`, which only references Vite's and Svelte's types, is the exception), the
   definite-assignment `!` (`let x!: T`), explicit type arguments on `querySelector`,
   `querySelectorAll`, `closest` and `getElementById` (narrow with `instanceof`), calls of
   `JSON.parse` and `.json()`, and a parameter on any rejection callback (`.catch(e => …)`,
   the second argument of `.then`): a rejection value is `any`, so the callback takes none.
3. *Independent check* (`dashboard/scripts/check-types.ts`, TypeScript compiler API, no
   ESLint involved): walks every source file, the generated bindings, the tests, the e2e
   specs and the config files, plus every `.svelte` file **whole** (parsed with
   `svelte/compiler`'s `parse`, so expressions in the markup are covered as well as the
   script blocks: Svelte 5 accepts `as` inside `{…}`), and fails on every construct the
   lint layer bans (the keyword nodes, assertions, non-null and definite `!`, type
   predicates, `declare`, `JSON.parse`), any `@ts-` directive comment, any `<script>`
   without `lang="ts"`, any `svelte-ignore` comment other than the one of 9.3, and any
   **tracked** `.js`, `.mjs` or `.cjs` file (`git ls-files`; build output is not source).
   It is syntax-tree based, so the word "never" in a sentence of `copy.ts` is not a finding.
   It also closes the hole syntax cannot see, an **inferred** `any`: over the `.ts` and
   `.svelte.ts` files it asks the type checker for the type of every expression and
   declaration and fails when one is `any` or `unknown` (this is how a third-party
   signature that returns `any`, such as `CustomEvent.detail` or `Object.fromEntries`, is
   caught without anyone writing the keyword); inside `.svelte` files the lint layer's
   `no-unsafe-*` rules report every *use* of an `any` value (call, member access,
   assignment, argument, return) but not one that is only printed in the markup. That
   rest is closed at the source: a `.svelte` file may import only `svelte`, other
   components, the project's own `.ts`/`.svelte.ts` modules and the generated bindings
   (`no-restricted-imports`; `check-types.ts` repeats the check), and every one of those
   modules is under the inferred-`any` check, so no third-party value typed `any` can
   enter a component except through a member access the lint reports.
   Configuration files are TypeScript where the tool loads TypeScript (`vite`, `vitest`,
   `playwright`, `eslint`) and JSON where it does not (`.stylelintrc.json`,
   `.prettierrc.json`: data, not code).

Patterns that make this workable:
- **No value of unknown type is ever created.** IPC results are `Outcome<T>` (7.3);
  `invoke` and `listen` are called with generated type arguments in `ipc/tauri.ts` only.
  That is the one trust boundary, and it is covered from the Rust side (the command tests
  compare the payload JSON with the bindings). The UI never calls `JSON.parse`, never reads
  a rejection value and has no `try`/`catch` outside `ipc/tauri.ts` and the clipboard helper,
  where the handlers take no parameter.
- **Exhaustiveness without `never`**: lookup tables typed
  `satisfies Record<Unavailable, Copy>` (a missing variant is a compile error) and the
  switch-exhaustiveness lint for tagged unions.
- **Narrowing instead of asserting**: DOM values are narrowed with `instanceof`; optional
  values with explicit checks; array reads go through `noUncheckedIndexedAccess`.
- Third-party declaration files in `node_modules` are not source and are outside the rule,
  but a value they type as `any` may not enter our code (the inferred-`any` check above).
- Explicit type arguments on `invoke` and `listen` are written only in `ipc/tauri.ts`; they
  are the one place the UI states a type the compiler cannot check, and the Rust command
  tests cover it from the other side.

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
  (hairlines, the chart); the 320 CSS px reflow runs of 9.10 (320 px at 100 %, and 640 px
  at 200 % emulated as 320 CSS px at ratio 2); a text-spacing run (the 1.4.12 values
  injected as a style sheet, in a browser context with `bypassCSP` for that run only);
  a focus-kept run (a snapshot that reorders cards while a meter row has focus); "no
  network" assertion. Browser:
  **Chromium** (present on this machine; Playwright's WebKit does not run on Arch). These
  baselines guard against regressions; they do not prove WebKitGTK renders the same, which
  is what the manual check list of T11 and T13 covers.
- **Real binary**: the driver-free smoke test of 11.4, and the `tauri-driver` +
  `WebKitWebDriver` end-to-end tests on the real WebKitGTK view (required, 1a).

## 11. Quality gates

"Zero bugs" cannot be promised by anyone. What can be promised is that every known class of
defect has a mechanical check, that the dangerous classes are unrepresentable in the types,
and that nothing merges without the gates below and a lead review.

### 11.1 Static
- Rust, always scoped to the three app crates (`P` = `-p jev-usage-core -p jev-usage-io
  -p jev-dashboard`; at the root a bare command selects the engine, which is not formatted
  with rustfmt today and has its own lint policy): `cargo fmt --check P`;
  `cargo clippy P --all-targets --locked -- -D warnings` with the `[lints]` table each app
  crate carries (5, invariant 3): `clippy::all`, `pedantic` (selected allow-list written in the manifest),
  `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `arithmetic_side_effects`,
  `as_conversions`, `print_stdout`, `print_stderr`, `dbg_macro`, `todo`, `unimplemented`
  denied outside tests; `unsafe_code` **forbidden in all three crates**; no
  `serde_json::Value` or `json!` outside tests (`clippy::disallowed_types`/`disallowed_macros`,
  configured in `dashboard/clippy.toml`, which clippy finds from each app crate's manifest
  directory upwards and which therefore never applies to the engine, and the architecture
  test); `missing_docs` warned on public items.
  **Exceptions written here once, so no worker has to ask** (the architecture test fails on
  any other `allow`): (1) *tests*: clippy's in-test switches cover only `#[test]` functions,
  not helpers, and `disallowed_types` has no such switch, so every file under `tests/` and
  every `#[cfg(test)] mod` starts with one fixed header,
  `allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing,
  clippy::arithmetic_side_effects, clippy::as_conversions, clippy::disallowed_types,
  clippy::disallowed_macros)`, character for character; (2) *one cast*: Rust has no checked
  conversion from a float to an integer, so `quantity.rs` has exactly one private function
  with `allow(clippy::as_conversions, clippy::cast_possible_truncation,
  clippy::cast_sign_loss)` that turns a finite, range-checked `f64` into `u32`;
  `Percent::from_fraction` is its only caller, and nothing else in the three crates
  converts a float to an integer (the tray rasteriser uses integer arithmetic, durations
  use `Duration::mul_f64` and `try_from`).
- TypeScript: the three layers of section 10 ("Type discipline"): `tsc --noEmit` and
  `svelte-check` with zero errors and zero warnings, ESLint (type-checked strict rules plus
  the security rules of 8.10, no inline configuration), `check-types.ts`; stylelint (token
  rule); Prettier check.
- Generated bindings up to date (`git diff --exit-code` after regeneration) and free of
  `any`/`unknown`/`never` (`check-types.ts` covers them).

### 11.2 Tests
| level | what | tool |
|---|---|---|
| unit | every pure function in core; every formatter | `cargo test`, Vitest |
| property | parsers never panic and round-trip; `Percent`/`Money`/`MeterId::key` laws; `alerts::step` invariants (≤ 1 alert per threshold per window instance for any reading sequence); `assess::derive` totality; `should_store`/`downsample` bounds; tail reader equals whole-file read for any chunking and any rotation point | `proptest` (default 256 cases; CI 4096) |
| fuzz | `parse::claude_log`, `parse::claude_event`, `parse::codex`, `parse::deepseek`, `Money::parse_decimal`, `MeterId::parse_key`, settings JSON | `cargo-fuzz` targets in `dashboard/fuzz/`, 60 s each, when a nightly toolchain is present; the same entry points are always exercised by a stable byte-level proptest so the gate never silently disappears |
| adapter | each source against recorded fixtures and failure cases (timeout, garbage, truncated, huge line, non-zero exit, 401, 429, 500, offline) using a stub program and a local stub HTTP server | `cargo test` |
| integration | service loop with fake clock, fake timer and fake sources: scheduling, backoff, visibility, suspend jump, clock set, alerts once, canary end to end | `cargo test` with `FakeClock`/`FakeTimer` (and tokio paused time for fetch timeouts) |
| component / screen / visual / a11y | section 10 | Vitest, Playwright, axe |
| real binary | smoke test without a driver; WebDriver e2e (required; the driver is installed here, 3.4; NOT RUN without it is never a pass, 1a) | 11.4 |
| security | section 8.10 | all of the above |

### 11.3 Coverage and budgets
- Line coverage (`cargo llvm-cov`): core ≥ 95 %, io ≥ 85 %; the shell is glue and is covered
  by e2e. Vitest: `src/lib` logic ≥ 95 %, components ≥ 85 %.
- Resource budgets, measured by `dashboard/scripts/measure-idle.sh` on the release build,
  started hidden under `dbus-run-session` against a temp tree with stub CLIs. The window
  measured is the 10 minutes from 60 s to 660 s after launch, which contains **no due
  fetch** (the start-up fetches are over and the next hidden Codex poll is at 15 min).
  Summed over the **whole process tree** (the app and any WebKit helper that is still alive)
  and over all threads (`/proc/<pid>/task/*/`): CPU time < 0.5 s, average wakeups (voluntary
  context switches) < 2 per minute, RSS ≤ 60 MB; binary ≤ 15 MB. The script also reports,
  without a budget, the CPU time and context switches of one stub Codex poll, and the same
  three figures for T00's empty scaffold started the same way (the floor that Tauri, GTK
  and the tray library cost before any of our code runs). That destroying the window really
  ends WebKit's helper processes is assumed (16) and this measurement is what checks it.
  There is no code fallback for either case: if the helpers stay, or the scaffold's floor
  alone is above a budget, the lead reports the measured numbers and the user decides
  whether the budget is restated; a budget is never met by changing how it is measured.

### 11.4 `scripts/ci.sh`
One script, fail-fast, prints a table of stages. Stages: (1) engine: `cargo build
--all-targets --locked` and `cargo test --locked` at the root (every existing engine test
must stay green; when the engine ships its own gate script, stage 1 runs that instead);
(2) `pnpm install --frozen-lockfile` and the UI build, then fmt and clippy over the three
app crates (11.1). The UI build comes first because the shell crate embeds `dashboard/dist`
at compile time (`generate_context!` fails when the directory is missing), so **every**
cargo command that compiles `jev-dashboard` needs it; (3) dashboard Rust tests incl. property tests at CI case
counts; (4) bindings drift; (5) `cargo deny check`, `cargo audit`; (6) UI lint, typecheck, `check-types.ts` (1a), unit and component tests (with
axe), `pnpm audit --prod`; (7) bundle budgets, from the stage-2 build; (8) Playwright screens, visual, accessibility,
keyboard, no-network; (9) release build (`--profile dist`) and manifest snapshot tests;
(10) **real-binary smoke test** (no driver needed): start the `dist` binary on the current
Wayland display but isolated from the user's session: under `dbus-run-session` with the
activation-free bus configuration (8.8), with
`HOME`, every `XDG_*_HOME` and `XDG_RUNTIME_DIR` pointing into a temp tree (0700) that holds
a synthetic `usage.jsonl`, `WAYLAND_DISPLAY` set to the **absolute path** of the real socket
(it is otherwise looked up in `XDG_RUNTIME_DIR`), and a `PATH` that contains only stub
`codex` and `claude` programs; read its stderr `diag` lines and assert `Started`, a
successful Claude-log and Codex fetch, `WindowReady` within budget (taken from the **second**
launch in the same temp tree: the first one builds the font and shader caches of an empty
home and is not what the user experiences), a new sample within 3 s
of an appended row, rows in `history.db`, no canary anywhere in the temp tree or on stderr,
no new file under the real state, config and data directories, and a clean exit on SIGTERM
with no child left; (11) coverage thresholds; (12) fuzz (60 s per
target, nightly); (13) WebDriver end-to-end on the real build (`tauri-driver` +
`WebKitWebDriver`).

Tools: `scripts/ci.sh --bootstrap` installs, at pinned versions, with `--locked`, and without
root: `cargo-deny`, `cargo-audit`, `cargo-llvm-cov`, `cargo-fuzz`, a pinned nightly toolchain
(through `rustup`, which is present), `tauri-driver`, and Playwright's Chromium (already
cached here). A stage whose tool is missing prints **SKIPPED (reason, how to install)**
loudly and the summary lists it; no stage is ever skipped silently. Stage 13 without
`WebKitWebDriver` prints **NOT RUN (the user installs it: `sudo pacman -S webkitgtk-6.0`)**,
because that one tool needs root. `scripts/ci.sh --release` treats SKIPPED **and NOT RUN**
as failures: per 1a the WebDriver stage is required, and NOT RUN is never a pass. The one
exception is stage 12: fuzzing needs a nightly toolchain and is **advisory** (its SKIPPED
is listed and does not fail `--release`), because the same seven entry points are always
run by the stable byte-level property tests of stage 3 (11.2). Stages 10
and 13 need a graphical session. Stage 13 runs the same isolated launch as stage 10 through
`tauri-driver`; that the 2.54.1 driver from `webkitgtk-6.0` drives a `webkit2gtk-4.1` app of
the same version, that automation works with the incognito web view, and that the driver
can inject axe into a page under the production CSP, are **assumed**
and checked the first time the stage runs (the fallback for the second is a non-incognito
web view in the WebDriver run only, with its data directory inside the temp tree; for the
third, the real view keeps its keyboard and content assertions and axe stays in stage 8).

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
For v1 as a whole, additionally: `ci.sh --release` green on this machine **with stage 13
passing** (NOT RUN is not green, 1a), the idle budgets **met** (or restated by the user,
11.3), a manual run beside a live `jev-claude` session for one day with no gateway
interference, which means all of: the app's only open descriptor in the gateway's state
directory outside `dashboard/` is `usage.jsonl`, read-only (`/proc/<pid>/fdinfo`, sampled
at start and end); owner and mode of every entry there are unchanged; `gateway.out` has
not grown an error the day before did not have (line counts by the lead's script, which
prints counts only); the user reports no failed turn; then the manual check lists of
T04, T11 and T13, and the **manual screen-reader pass with Orca** over every screen and
state: the lead supplies the written script (what must be heard, in order), the user runs
Orca and confirms each line or reports what was said instead; every difference is a finding
and v1 is not done while one is open (1a).

## 12. Packaging, install, versioning
- Build: `pnpm --dir dashboard install --frozen-lockfile && pnpm --dir dashboard tauri build
  --no-bundle -- --profile dist --locked`. The result is one binary,
  `target/dist/jev-dashboard`, which needs only the system's webkit2gtk-4.1, gtk3 and
  libayatana-appindicator (all present).
- `dashboard/scripts/install.sh [dest]` (default `~/.local/bin`): installs the binary as
  `jev-dashboard` (written beside the target and renamed over it, so a running instance
  keeps its old file and is never half-replaced), `dev.jev.flashboard.desktop` (`Name=Flashboard`, `Exec=` the absolute
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
  blocked in the kernel. The watched directory also holds the gateway's other logs; the
  directory watch asks only for create, move and delete events and the modify watch is on
  the one log file, so writes to the other logs wake nothing (7.2).
- A burst of gateway writes costs one 1.5 s-coalesced tail read of the new bytes only, with a
  cheap byte pre-filter (`"ratelimit":{"`, which a row with an empty `ratelimit` object
  does not contain) before JSON parsing.
- A Codex poll is one short-lived process (about 2.7 s wall, almost all waiting) every
  5/15 minutes; nothing stays resident. Through the mise wrapper each start also costs
  mise's own launch; its self-update is switched off for our children (8.6).
- Snapshots are emitted only on change. Tray icons are cached and the tray is touched only
  when its own content changes. History queries return at most 600 points, aggregated in SQL
  (7.5), so memory per query is bounded by 600 points whatever the range.
- The login probe (two short CLI starts) runs at start and then only when needed (7.2),
  never on a fixed short timer.
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
| `WebKitWebDriver` is not in Arch's `webkit2gtk-4.1` | resolved here | the user has installed `webkitgtk-6.0`, which ships it (1a, 3.4); if that driver cannot drive the 4.1 app the lead reports it and the user decides |
| The account wrapper runs an explicit `mise install` or `mise use -g` (a write to the user's mise configuration) when it finds no real binary (3.1) | low | 8.6: a tool counts as installed only with its executable on disk, so the fallback is not reached; `MISE_OFFLINE=1` blocks a download; T04 compares a hash of the mise configuration before and after |
| The hidden app's floor (Tauri, GTK, tray library) is above the idle budget before our code runs | unknown | `measure-idle.sh` reports the empty scaffold's figures beside the app's; the user decides on a restated budget (11.3) |
| A panic in a dependency ends the resident app until the next login | low | unwinding kept in `dist` so a task panic is contained (5); own code cannot panic by lint; main-thread panics remain fatal and are stated |
| `PR_SET_DUMPABLE=0` upsets a portal, WebKit's sandbox helper or its accessibility bridge | low | checked by hand in T11; fallback is `RLIMIT_CORE=0` only (8.8) |
| A session `PATH` without `codex`/`claude` (autostart on another setup) | low here (verified, 3.4) | exact copy on the card, README section, never guess directories |
| A mise wrapper starts an install or the four-hourly self-update when we run `codex`/`claude` (both are `lazy`, `auto_update = "4h"` here) | medium | 8.6: install-on-disk check before any start; `MISE_OFFLINE=1` and the three auto-install switches in the child environment; process-group kill as backstop; before/after listing check in T04 |
| The scrubbed child environment breaks `codex app-server` or `claude` (login or account wrapper not found; `MISE_OFFLINE` refused) | medium | allow-list includes the home, bus and tool-home variables (8.6); one real, free Codex run and both status commands under the scrubbed environment in T04's manual list |
| `notify-rust`/`zbus` API or version drift | low | pinned; one module each; the Notifier and tray-host checks are behind traits with fakes |
| The engine library drags heavy dependencies into the app (Path A) | certain, low impact | link-time removal; the refactor's facade crate removes it |
| The engine refactor (one crate per context, a facade crate) lands before, during or after T00 | certain | 5 and 5.1: one port, two lines, contract tests; both orders are specified; Flashboard's lints and clippy file are its own |
| Web view memory when open (WebKit processes, 100 MB+) | certain | web view destroyed when the window is closed; that its helper processes exit is measured by `measure-idle.sh` over the process tree |
| DeepSeek adapter unverified until a key exists | certain | built against the documented shape with fixtures; marked unverified in Diagnostics until first success |
| Shared `Cargo.lock` churn for the engine | low | `default-members`, `--locked`, engine tests in stage 1 |

## 15. Designing for the later chat window (not built)
- The usage side keeps reaching the engine only through `io::engine_bridge`. The chat
  feature gets its **own** port of the same kind in a `jev-chat-io` crate beside
  `jev-usage-io` (turn execution, events), over the same engine facade. The usage service
  is unaffected.
- Turns run by the chat engine yield `PlanUsage`/rate-limit events; a `ChatTurnSource`
  implementing `UsageSource` with `Trigger::OnSignal` keeps the latest event and signals the
  service through the same `signals` channel the file watcher uses, so Claude usage becomes
  live while chatting with no new UI and **no change to the trait, the service or the IPC**.
- `Notifier`, `HistoryStore`, `SettingsStore`, `WallTimer` and `LoginProbe` are unaffected by
  chat. `Settings` gains fields through its `version` and migration, not a new store.
- The IPC rule changes for chat (prompts and answers must cross). That will be a **separate
  window label with a separate capability file and separate commands**, so the dashboard
  window's "numbers only" guarantee and its tests stay intact.
- The shell's window manager already creates and destroys windows by label.

## 16. Verified versus assumed
**Verified** (section 3): Codex app-server transport, sequence, timings, exit behaviour and
free-plan shapes; `usage.jsonl` field names, value types, freshness and line size; tray and
notification hosts present; Tauri CLI `info` output; toolchain versions; which tools are
missing; `gemini --help` has no usage command; `codex`/`claude` are lazy, self-updating mise
wrappers that select the account, which stream each status command answers on, and mise's
environment switches (3.1); `webkitgtk-6.0` ships `WebKitWebDriver` (3.4);
`ts_ms` order, the `ratelimit` key set and the sporadic `7d_oi` window (3.2); log rotation by
rename; XDG autostart is honoured by this session and its `PATH` finds `codex` and `claude`
(3.4); what the engine's probe reads and runs, which is why it is not called (5.1).

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
`webkitgtk-6.0` package can drive a `webkit2gtk-4.1` app, also with an incognito web view;
the four `MISE_*` switches stop every download **and every configuration write** when a
wrapper runs here (documentation says offline stops downloads and updates; the wrapper
script's `mise use -g` is kept unreachable by the executable check); `codex app-server`
accepts its three request lines at once; `WebviewUrl::App` keeps a
URL fragment; Svelte 5 runs under `freezePrototype`; Tauri's macros compile under
`#![forbid(unsafe_code)]` and pass `disallowed_types`; `svelte-check` and the ESLint Svelte
plugin work without a `svelte.config.js`; WebKitGTK paints nothing before the entry module
has set the theme; the WebDriver session can inject axe under the CSP; the scaffold's idle
floor is inside the budgets; the `inotify` crate's descriptor and a `rustix` timerfd can be
driven by tokio's `AsyncFd` under `forbid(unsafe_code)` (T03, T08); the tray host shows the
item title as a name, not a panel label (T11); GTK and WebKitGTK start on a session bus
that activates nothing (T12, stage 10); `DISABLE_AUTOUPDATER=1` is honoured by `claude`
(T04); in Path B the engine's facade crate offers the six names of 5.1 (T00);
`generate_context!` needs the built UI at compile time (T00: if it does not, stage 2's
order is merely harmless); Tauri does not apply `csp` to the Vite dev server's page (T00,
else `devCsp`, 8.8); WebKitGTK exposes `role="meter"` with
`aria-valuetext` to Orca. Each of these has a named check in
the task that depends on it, and none blocks the design if it turns out false: the fallback
is written where the assumption is used, or, for the two idle-budget assumptions and the
`forbid`/lint ones, the stated outcome is a report to the user and not a silent relaxation.

## 17. Open questions
Answered by the user on 2026-11-17 (section 1a): the name, the Claude live check, start at
login, the account e-mail, what comes after v1, and the UI quality bar.

Also answered on 2026-11-17 (1a): the user installs `WebKitWebDriver` (`webkitgtk-6.0`) and
Orca (both now installed, 3.4), the WebDriver stage is required, and the user takes part in
the screen-reader pass.
The interrupted `antigravity-cli` directory has been removed by the lead.

Still for the user: none.

For the lead (resolved during implementation, not user-owned): which path of 5 applies when
T00 starts (the engine refactor's schedule decides; both are specified); moving the engine's own `probe_all` onto the narrow probe of 8.6 (it has the
same wrapper and token-file behaviour, outside this project's scope); whether to store a
last reading for instant cold-start display; PKGBUILD.
