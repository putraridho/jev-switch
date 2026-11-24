//! Domain to IPC read model (DESIGN 7.4). T02 writes the function bodies.

use crate::ipc::{RefreshView, Snapshot};
use crate::provider::ProviderKey;
use crate::settings::Settings;
use crate::time::UnixMillis;
use crate::usage::Usage;

/// One provider as the service holds it, ready to be turned into its view.
#[derive(Clone, PartialEq, Debug)]
pub struct ProviderState<'a> {
    /// The provider.
    pub provider: ProviderKey,
    /// Its derived state.
    pub usage: &'a Usage,
    /// Its refresh bookkeeping.
    pub refresh: RefreshView,
}

/// The whole read model the UI and the tray render from.
pub fn snapshot(states: &[ProviderState<'_>], now: UnixMillis, settings: &Settings) -> Snapshot {
    let _ = (states, settings);
    Snapshot {
        generated_at: now.0,
        providers: Vec::new(),
        headline: None,
    }
}

/// Whether two snapshots show the same thing; ignores `generated_at`.
pub fn same_content(a: &Snapshot, b: &Snapshot) -> bool {
    let _ = (a, b);
    false
}
