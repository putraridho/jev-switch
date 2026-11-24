//! Ports: the traits core defines and io implements (DESIGN 4.4).

use crate::alerts::{Alert, AlertMemory};
use crate::history::{SeriesData, StoreStats};
use crate::ipc::HourCycle;
use crate::meter::{MeterId, Reading};
use crate::provider::ProviderKey;
use crate::settings::Settings;
use crate::time::UnixSeconds;
use crate::usage::{Setup, Unavailable};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// One way of learning one provider's usage.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub enum SourceId {
    /// The local gateway's usage log.
    #[serde(rename = "claude_gateway_log")]
    ClaudeGatewayLog,
    /// The opt-in live check through `claude -p`.
    #[serde(rename = "claude_live")]
    ClaudeLive,
    /// The `codex app-server` protocol.
    #[serde(rename = "codex_app_server")]
    CodexAppServer,
    /// The DeepSeek balance endpoint.
    #[serde(rename = "deepseek_balance")]
    DeepSeekBalance,
    /// Gemini: no source yet.
    #[serde(rename = "gemini_none")]
    GeminiNone,
    /// Antigravity: no source yet.
    #[serde(rename = "antigravity_none")]
    AntigravityNone,
    /// Grok: no source yet.
    #[serde(rename = "grok_none")]
    GrokNone,
}

/// When the service fetches a source.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Trigger {
    /// Fetch when the source signals that it has news (plus once at start and when the
    /// window becomes visible). The gateway log's file watcher signals; the later chat
    /// window's turn events signal the same way, with no change to this trait.
    OnSignal,
    /// Fetch on a timer; the interval depends on whether the window is visible.
    Interval {
        /// The interval while the window is visible.
        visible: Duration,
        /// The interval while it is hidden.
        hidden: Duration,
    },
    /// Fetch only when the user asks.
    Manual,
}

/// Whether a fetch spends the user's quota.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Cost {
    /// Costs nothing.
    Free,
    /// Spends a (small) request of the user's plan.
    SpendsQuota,
}

/// Everything the service needs to know about a source; it never matches on the id.
#[derive(Copy, Clone, Debug)]
pub struct SourceDescriptor {
    /// The source.
    pub id: SourceId,
    /// The provider it reports on.
    pub provider: ProviderKey,
    /// When it is fetched.
    pub trigger: Trigger,
    /// Whether a fetch spends quota.
    pub cost: Cost,
    /// The deadline of one fetch.
    pub timeout: Duration,
    /// How long a reading of this source counts as current (DESIGN 7.7).
    pub fresh_for: Duration,
    /// True when the source starts a CLI that needs a login: the service probes the login
    /// first when it is not known `Ready` (DESIGN 7.2).
    pub needs_login: bool,
}

/// Why a fetch produced no reading.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum SourceError {
    /// The user has to set something up first. An answer, not a failure.
    NotConfigured(Setup),
    /// The fetch failed. The only variant that counts towards backoff.
    Unavailable(Unavailable),
    /// The provider has no source. An answer, not a failure.
    Unsupported,
}

/// The future a fetch returns.
pub type FetchFuture<'a> = Pin<Box<dyn Future<Output = Result<Reading, SourceError>> + Send + 'a>>;

/// One way to learn a provider's usage. Adding a provider is adding one implementation and
/// one line in the registry; nothing else changes (open/closed).
pub trait UsageSource: Send + Sync {
    /// What the service needs to know to schedule this source.
    fn descriptor(&self) -> SourceDescriptor;
    /// Reads the provider's usage once.
    fn fetch(&self) -> FetchFuture<'_>;
}

/// Why the store could not do what was asked.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum StoreError {
    /// The store cannot be used now (locked, read-only, missing).
    Unavailable,
    /// The stored data is damaged.
    Corrupt,
}

/// The history of readings and the alert memory. Synchronous and does file IO: the service
/// calls every method through `spawn_blocking`.
pub trait HistoryStore: Send + Sync {
    /// Stores the samples of one reading under the write rule (DESIGN 7.5).
    fn record(&self, provider: ProviderKey, reading: &Reading) -> Result<(), StoreError>;
    /// Many readings in one transaction (the first-run backfill). Returns samples stored.
    fn record_all(&self, provider: ProviderKey, readings: &[Reading]) -> Result<u64, StoreError>;
    /// Points, reset marks, bucket width. A span (`to - from`) longer than
    /// `history::RAW_RETENTION` is a rollup query (DESIGN 7.5); the store is not told the
    /// range name.
    fn series(
        &self,
        provider: ProviderKey,
        meter: &MeterId,
        from: UnixSeconds,
        to: UnixSeconds,
        max_points: u16,
    ) -> Result<SeriesData, StoreError>;
    /// The persisted alert memory.
    fn load_alerts(&self) -> Result<AlertMemory, StoreError>;
    /// Replaces the persisted alert memory.
    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError>;
    /// Applies retention as of `now`.
    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError>;
    /// File bytes and sample count, for Diagnostics.
    fn stats(&self) -> Result<StoreStats, StoreError>;
}

/// The user's settings. Synchronous and does file IO, like [`HistoryStore`].
pub trait SettingsStore: Send + Sync {
    /// The stored settings; defaults when missing or invalid.
    fn load(&self) -> Settings;
    /// Replaces the stored settings.
    fn save(&self, s: &Settings) -> Result<(), StoreError>;
}

/// Shows an alert to the user. MUST NOT block: an implementation hands the alert to its own
/// worker and returns.
pub trait Notifier: Send + Sync {
    /// Shows `alert`; `hour` is the current hour-cycle setting, passed by the service.
    fn notify(&self, alert: &Alert, hour: HourCycle);
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use static_assertions::assert_obj_safe;

    assert_obj_safe!(UsageSource, HistoryStore, SettingsStore, Notifier);

    #[test]
    fn source_id_names() {
        let cases = [
            (SourceId::ClaudeGatewayLog, "claude_gateway_log"),
            (SourceId::ClaudeLive, "claude_live"),
            (SourceId::CodexAppServer, "codex_app_server"),
            (SourceId::DeepSeekBalance, "deepseek_balance"),
            (SourceId::GeminiNone, "gemini_none"),
            (SourceId::AntigravityNone, "antigravity_none"),
            (SourceId::GrokNone, "grok_none"),
        ];
        for (id, name) in cases {
            assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn errors_are_field_less_or_closed_enums() {
        assert_eq!(
            format!("{:?}", SourceError::Unavailable(Unavailable::Timeout)),
            "Unavailable(Timeout)"
        );
        assert_eq!(
            format!("{:?}", SourceError::NotConfigured(Setup::AddDeepSeekKey)),
            "NotConfigured(AddDeepSeekKey)"
        );
        assert_eq!(format!("{:?}", StoreError::Corrupt), "Corrupt");
    }
}
