//! Which (provider, model) pairs a turn may be routed to.

use super::{Availability, State};
use jev_backend::ProviderId;

/// The (provider, model) pairs a turn may be routed to: for each Ready provider, the
/// models its login lists, else everything `all_models` knows for it. Providers that
/// are not Ready contribute nothing, so routing can never pick one. The catalog is
/// injected so this stays pure.
pub fn usable_models(avail: &[Availability], all_models: &dyn Fn(ProviderId) -> Vec<String>) -> Vec<(ProviderId, String)> {
    avail
        .iter()
        .filter(|a| a.state == State::Ready)
        .flat_map(|a| {
            let models = a.models.clone().unwrap_or_else(|| all_models(a.provider));
            models.into_iter().map(move |m| (a.provider, m))
        })
        .collect()
}
