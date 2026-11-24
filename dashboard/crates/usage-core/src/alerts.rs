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
    let mut next = memory.clone();
    let mut alerts = Vec::new();
    if settings.hidden_providers.contains(&provider)
        || now.0.saturating_sub(reading.observed_at.0) > 600_000
    {
        return (next, alerts);
    }
    for meter in reading.meters.as_slice() {
        let crate::meter::Meter::Window(window) = meter else {
            continue;
        };
        if crate::assess::phase(window, now) == crate::assess::Phase::Elapsed {
            continue;
        }
        let id = meter.id();
        let thresholds = settings.thresholds();
        let levels = [
            (AlertLevel::Warn, thresholds.warn(), settings.notify_warn),
            (
                AlertLevel::Critical,
                thresholds.critical(),
                settings.notify_critical,
            ),
        ];
        let mut crossed = Vec::new();
        for (level, threshold, enabled) in levels {
            next.entries.retain(|entry| {
                entry.provider != provider
                    || entry.meter != id
                    || entry.level != level
                    || (entry
                        .window_resets_at
                        .is_none_or(|reset| reset.0.saturating_mul(1_000) > now.0)
                        && window.used.basis_points()
                            >= threshold.basis_points().saturating_sub(500))
            });
            if window.used >= threshold
                && !next.entries.iter().any(|entry| {
                    entry.provider == provider && entry.meter == id && entry.level == level
                })
            {
                next.entries.push(AlertEntry {
                    provider,
                    meter: id.clone(),
                    level,
                    fired_at: UnixSeconds(now.0.checked_div(1_000).unwrap_or(0)),
                    window_resets_at: window.resets_at,
                });
                crossed.push((level, enabled));
            }
        }
        if let Some((level, enabled)) = crossed.last() {
            if settings.notifications && *enabled {
                alerts.push(Alert {
                    provider,
                    window: window.kind.clone(),
                    level: *level,
                    used: window.used,
                    resets_at: window.resets_at,
                });
            }
        }
    }
    (next, alerts)
}
