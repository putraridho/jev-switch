//! Freshness, phase and severity: pure policy (DESIGN 4.3, 4.5). The types are frozen;
//! T02 writes the function bodies.

use crate::meter::{Meter, Reading, WindowReading};
use crate::port::SourceError;
use crate::settings::Thresholds;
use crate::time::UnixMillis;
use crate::usage::{StaleCause, Usage};
use serde::Serialize;
use std::time::Duration;

/// How long a reading counts as current: the `fresh_for` of the source that produced it.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct FreshnessPolicy {
    /// The longest age of a current reading.
    pub fresh_for: Duration,
}

/// What the service remembers per provider (the merge of its sources).
#[derive(Clone, PartialEq, Debug)]
pub struct SourceState {
    /// The newest good reading, if any.
    pub last_good: Option<Reading>,
    /// The newest failure and when it happened, if any.
    pub last_failure: Option<(UnixMillis, SourceError)>,
    /// Whether any source of the provider has been asked in this run.
    pub attempted: bool,
}

/// Whether a window is still running.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The window has not reset yet (or says nothing about a reset).
    Current,
    /// The window's reset time has passed; its number describes a window that ended.
    Elapsed,
}

/// How a meter stands against the user's thresholds.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Below the warn level.
    Ok,
    /// At or above the warn level.
    Warn,
    /// At or above the critical level (or a balance that cannot be used).
    Critical,
    /// Nothing can be said (an elapsed window).
    Unknown,
}

/// The provider's state, derived from what the service remembers (DESIGN 4.3).
pub fn derive(state: &SourceState, now: UnixMillis, policy: &FreshnessPolicy) -> Usage {
    if let Some(last) = &state.last_good {
        let expires = last
            .observed_at
            .0
            .saturating_add(u64::try_from(policy.fresh_for.as_millis()).unwrap_or(u64::MAX));
        let failure = state.last_failure.filter(|(at, error)| {
            at.0 >= last.observed_at.0 && matches!(error, SourceError::Unavailable(_))
        });
        if let Some((at, SourceError::Unavailable(reason))) = failure {
            return Usage::Stale {
                last: last.clone(),
                since: UnixMillis(at.0.min(expires)),
                cause: StaleCause::Failing { reason },
            };
        }
        return if now.0 <= expires {
            Usage::Known(last.clone())
        } else {
            Usage::Stale {
                last: last.clone(),
                since: UnixMillis(expires),
                cause: StaleCause::Quiet,
            }
        };
    }
    match state.last_failure {
        Some((_, SourceError::Unavailable(reason))) => Usage::Unavailable { reason },
        Some((_, SourceError::NotConfigured(setup))) => Usage::NotConfigured { setup },
        Some((_, SourceError::Unsupported)) => Usage::Unsupported,
        None => Usage::Pending,
    }
}

/// `Elapsed` when the window's `resets_at` is not after `now`.
pub fn phase(w: &WindowReading, now: UnixMillis) -> Phase {
    if w.resets_at
        .is_some_and(|at| at.0.saturating_mul(1_000) <= now.0)
    {
        Phase::Elapsed
    } else {
        Phase::Current
    }
}

/// A window by its `used` against the thresholds, `Unknown` when elapsed; a balance is
/// `Critical` when not usable, else `Ok` (DESIGN 4.5).
pub fn severity(m: &Meter, now: UnixMillis, t: &Thresholds) -> Severity {
    match m {
        Meter::Window(w) if phase(w, now) == Phase::Elapsed => Severity::Unknown,
        Meter::Window(w) if w.used >= t.critical() => Severity::Critical,
        Meter::Window(w) if w.used >= t.warn() => Severity::Warn,
        Meter::Balance(b) if !b.usable => Severity::Critical,
        Meter::Window(_) | Meter::Balance(_) => Severity::Ok,
    }
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

    #[test]
    fn phase_and_severity_names() {
        assert_eq!(
            serde_json::to_string(&Phase::Current).unwrap(),
            "\"current\""
        );
        assert_eq!(
            serde_json::to_string(&Phase::Elapsed).unwrap(),
            "\"elapsed\""
        );
        let names: Vec<String> = [
            Severity::Ok,
            Severity::Warn,
            Severity::Critical,
            Severity::Unknown,
        ]
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect();
        assert_eq!(names, ["\"ok\"", "\"warn\"", "\"critical\"", "\"unknown\""]);
    }
}
