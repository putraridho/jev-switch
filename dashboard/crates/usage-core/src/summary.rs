//! The headline: the one number the tray shows (DESIGN 4.5). T02 writes `headline`.

use crate::assess::Severity;
use crate::meter::MeterId;
use crate::provider::ProviderKey;
use crate::quantity::Percent;
use crate::settings::Settings;
use crate::time::{UnixMillis, UnixSeconds};
use crate::usage::Usage;

/// The window meter with the highest usage (the domain value; `view::snapshot` maps it to
/// the IPC `Headline`).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Headline {
    /// The provider it belongs to.
    pub provider: ProviderKey,
    /// The meter.
    pub meter: MeterId,
    /// How much is used.
    pub used: Percent,
    /// Its level against the thresholds.
    pub severity: Severity,
    /// When the window resets, if known.
    pub resets_at: Option<UnixSeconds>,
}

/// The highest non-elapsed window among `Known` and `Stale` readings of enabled providers.
pub fn headline(
    providers: &[(ProviderKey, &Usage)],
    now: UnixMillis,
    settings: &Settings,
) -> Option<Headline> {
    let _ = (providers, now, settings);
    None
}
