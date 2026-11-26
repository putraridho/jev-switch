//! IPC types: exactly what crosses to the web UI (DESIGN 7.4).
//!
//! Every field is a number, a boolean, a closed enum, a [`Label`], a [`MeterId`] (as its
//! key) or the compile-time `app_version`. There is no field that could hold a prompt, key,
//! token, e-mail, account id, path, hostname, URL or error message. TypeScript is generated
//! from these types (`--features bindings`); `u64`/`i64` fields say `number` because every
//! value is far below 2^53.

use crate::label::Label;
use crate::meter::MeterId;
use serde::{Deserialize, Serialize};

pub use crate::assess::{Phase, Severity};
pub use crate::meter::{LimitStatus, WindowKind};
pub use crate::port::SourceId;
pub use crate::provider::ProviderKey;
pub use crate::quantity::Currency;
pub use crate::settings::{Settings, SettingsPatch};
pub use crate::usage::{Setup, StaleCause, Unavailable};

/// One meter as the UI shows it.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeterView {
    /// A usage window.
    Window {
        /// The meter's key.
        #[cfg_attr(feature = "bindings", ts(type = "string"))]
        id: MeterId,
        /// Which window.
        window: WindowKind,
        /// How much is used, in basis points.
        used_bp: u16,
        /// When it resets (epoch seconds), if known.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        resets_at: Option<u64>,
        /// The provider's verdict.
        status: LimitStatus,
        /// Its level against the thresholds.
        severity: Severity,
        /// Whether the window is still running.
        phase: Phase,
    },
    /// A balance.
    Balance {
        /// The meter's key.
        #[cfg_attr(feature = "bindings", ts(type = "string"))]
        id: MeterId,
        /// The currency.
        currency: Currency,
        /// The whole balance, in millionths.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        total_micros: i64,
        /// The granted part, in millionths, if the provider splits it.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        granted_micros: Option<i64>,
        /// The paid part, in millionths, if the provider splits it.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        topped_up_micros: Option<i64>,
        /// Whether the balance can be spent now.
        usable: bool,
        /// `critical` when not usable, else `ok`.
        severity: Severity,
    },
}

/// One reading as the UI shows it.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ReadingView {
    /// When the numbers were observed (epoch milliseconds).
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub observed_at: u64,
    /// The plan name, when known.
    #[cfg_attr(feature = "bindings", ts(type = "string | null"))]
    pub plan: Option<Label>,
    /// The meters.
    pub meters: Vec<MeterView>,
}

/// One provider's state as the UI shows it.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UsageView {
    /// No source has answered yet.
    Pending,
    /// A current reading.
    Known {
        /// The reading.
        reading: ReadingView,
    },
    /// The last good reading, no longer current.
    Stale {
        /// The reading.
        reading: ReadingView,
        /// When it stopped being current (epoch milliseconds).
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        since: u64,
        /// Why it is not current.
        cause: StaleCause,
    },
    /// No reading to show.
    Unavailable {
        /// Why.
        reason: Unavailable,
    },
    /// The user has to do something first.
    NotConfigured {
        /// What.
        setup: Setup,
    },
    /// The app has no source for this provider.
    Unsupported,
}

/// A provider's refresh bookkeeping.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct RefreshView {
    /// A fetch is running.
    pub in_flight: bool,
    /// When the last fetch ended (epoch milliseconds), if any.
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub last_attempt_at: Option<u64>,
    /// When the next fetch is due (epoch milliseconds), if scheduled.
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub next_at: Option<u64>,
    /// The provider has a free source that a manual refresh starts.
    pub free_refresh: bool,
    /// The provider has a spending source AND the setting is on.
    pub live_check: bool,
    /// Why the last live check failed; `None` after a success.
    pub live_check_error: Option<Unavailable>,
}

/// One provider in the snapshot.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ProviderView {
    /// The provider.
    pub provider: ProviderKey,
    /// False when the user has hidden it.
    pub enabled: bool,
    /// Its state.
    pub usage: UsageView,
    /// Its refresh bookkeeping.
    pub refresh: RefreshView,
}

/// The one number the tray shows.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct Headline {
    /// The provider it belongs to.
    pub provider: ProviderKey,
    /// The meter's key.
    #[cfg_attr(feature = "bindings", ts(type = "string"))]
    pub meter_id: MeterId,
    /// How much is used, in basis points.
    pub used_bp: u16,
    /// Its level against the thresholds.
    pub severity: Severity,
    /// When the window resets (epoch seconds), if known.
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub resets_at: Option<u64>,
}

/// The whole read model.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct Snapshot {
    /// When it was built (epoch milliseconds).
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub generated_at: u64,
    /// Every provider, in `ProviderKey::ALL` order.
    pub providers: Vec<ProviderView>,
    /// The highest window, if any provider has one.
    pub headline: Option<Headline>,
}

/// What a manual refresh starts.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefreshTarget {
    /// Every free source.
    All,
    /// The free sources of one provider.
    Provider {
        /// The provider.
        provider: ProviderKey,
    },
    /// The spending source of one provider: the only target that may spend.
    LiveCheck {
        /// The provider.
        provider: ProviderKey,
    },
}

/// The span of a history query.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HistoryRange {
    /// The last 24 hours.
    Day,
    /// The last 7 days.
    Week,
    /// The last 30 days.
    Month,
    /// The last 90 days.
    Quarter,
}

/// Which series to read.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct HistoryQuery {
    /// The provider.
    pub provider: ProviderKey,
    /// The meter's key.
    #[cfg_attr(feature = "bindings", ts(type = "string"))]
    pub meter_id: MeterId,
    /// The span.
    pub range: HistoryRange,
}

/// The unit of a series' values.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SeriesUnit {
    /// Usage in basis points (a window).
    BasisPoints,
    /// Money in millionths (a balance).
    Micros,
}

/// One point of a series.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct HistoryPoint {
    /// The time (epoch seconds).
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub t: u64,
    /// The value.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub v: i64,
    /// The lowest value in the point's bucket.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub lo: i64,
    /// The highest value in the point's bucket.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub hi: i64,
}

/// A meter's history over a range: at most 600 points.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct HistorySeries {
    /// The provider.
    pub provider: ProviderKey,
    /// The meter's key.
    #[cfg_attr(feature = "bindings", ts(type = "string"))]
    pub meter_id: MeterId,
    /// The start of the range (epoch seconds).
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub from: u64,
    /// The end of the range (epoch seconds).
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub to: u64,
    /// The unit of the values.
    pub unit: SeriesUnit,
    /// The points, oldest first.
    pub points: Vec<HistoryPoint>,
    /// A hole longer than this many seconds is drawn as a gap.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub gap_s: u64,
    /// Window ends inside the range (epoch seconds), at most 64; may be empty.
    #[cfg_attr(feature = "bindings", ts(type = "number[]"))]
    pub resets: Vec<u64>,
}

/// The colour scheme.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follow the desktop.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// How hours are written.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HourCycle {
    /// Follow the locale.
    Auto,
    /// 12-hour clock.
    H12,
    /// 24-hour clock.
    H24,
}

/// Why a command could not do what was asked. A closed list: no command returns a string.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CommandError {
    /// An argument was malformed or out of range.
    InvalidArgument,
    /// The same work is already running.
    Busy,
    /// The feature is switched off (or the provider has none).
    NotEnabled,
    /// The history store cannot be used.
    StoreUnavailable,
    /// A file could not be written.
    Io,
    /// A fault inside the app.
    Internal,
}

/// The envelope every command resolves with, on success and on failure (DESIGN 7.3).
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome<T> {
    /// The command did what was asked.
    Ok {
        /// Its result.
        value: T,
    },
    /// The command refused or failed.
    Error {
        /// Why.
        error: CommandError,
    },
}

impl<T> From<Result<T, CommandError>> for Outcome<T> {
    fn from(result: Result<T, CommandError>) -> Self {
        match result {
            Ok(value) => Self::Ok { value },
            Err(error) => Self::Error { error },
        }
    }
}

/// One source in Diagnostics.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct SourceDiag {
    /// The source.
    pub source: SourceId,
    /// Its provider.
    pub provider: ProviderKey,
    /// When it last succeeded (epoch milliseconds); `None` when it never has.
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub last_ok_at: Option<u64>,
    /// Its last failure, if the last attempt failed.
    pub last_error: Option<Unavailable>,
    /// Consecutive failures.
    pub failures: u32,
}

/// What the Settings screen's diagnostics section shows.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct Diagnostics {
    /// The app's version: a compile-time constant.
    pub app_version: &'static str,
    /// Every source.
    pub sources: Vec<SourceDiag>,
    /// The history database's size in bytes.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub history_bytes: u64,
    /// The number of stored samples.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub history_samples: u64,
    /// Whether a tray host is present now.
    pub tray_available: bool,
}

/// A screen of the UI.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum Route {
    /// The overview.
    Overview,
    /// One provider's detail.
    Provider {
        /// The provider.
        provider: ProviderKey,
    },
    /// Settings.
    Settings,
}
