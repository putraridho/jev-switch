//! Threshold alerts: a pure state machine (DESIGN 7.6). The types are frozen; T02 writes
//! `step`.

use crate::meter::{MeterId, Reading, WindowKind};
use crate::provider::ProviderKey;
use crate::quantity::Percent;
use crate::settings::Settings;
use crate::time::{UnixMillis, UnixSeconds};

/// Which threshold was crossed.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum AlertLevel {
    /// The warn level.
    Warn,
    /// The critical level.
    Critical,
}

/// One notification to show.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Alert {
    /// The provider.
    pub provider: ProviderKey,
    /// The window that crossed.
    pub window: WindowKind,
    /// The level it crossed.
    pub level: AlertLevel,
    /// How much is used.
    pub used: Percent,
    /// When the window resets, if known.
    pub resets_at: Option<UnixSeconds>,
}

/// One remembered crossing: no second alert for it until it re-arms.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct AlertEntry {
    /// The provider.
    pub provider: ProviderKey,
    /// The meter that crossed.
    pub meter: MeterId,
    /// The level it crossed.
    pub level: AlertLevel,
    /// When it was recorded.
    pub fired_at: UnixSeconds,
    /// The reset time of the window instance it belongs to, if known.
    pub window_resets_at: Option<UnixSeconds>,
}

/// Every remembered crossing. Persisted, so a restart never repeats a notification.
#[derive(Clone, Eq, PartialEq, Debug, Default)]
pub struct AlertMemory {
    /// The crossings, at most one per `(provider, meter, level)`.
    pub entries: Vec<AlertEntry>,
}

/// One step of the alert machine for a new reading: the next memory and the alerts to show.
pub fn step(
    memory: &AlertMemory,
    provider: ProviderKey,
    reading: &Reading,
    now: UnixMillis,
    settings: &Settings,
) -> (AlertMemory, Vec<Alert>) {
    let _ = (provider, reading, now, settings);
    (memory.clone(), Vec::new())
}
