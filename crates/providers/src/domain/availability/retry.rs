//! The slow-probe retry and the merging of late or refreshed answers.

use std::future::Future;
use std::time::Duration;

use super::{capped, Availability, State};
use jev_backend::ProviderId;

/// A probe that ended [`State::Unknown`] is tried once more, in the background, with this
/// longer cap. `codex` and `claude` are often lazy wrappers (mise) whose first run can take
/// longer than [`PROBE_CAP`] while they install or warm up; waiting that long at startup
/// would stall the banner, and never asking again would leave the provider unused all session.
pub const RETRY_CAP: Duration = Duration::from_secs(20);

/// The providers whose first probe could not tell, and so deserve the retry.
pub fn retry_candidates(first: &[Availability]) -> Vec<ProviderId> {
    first.iter().filter(|a| a.state == State::Unknown).map(|a| a.provider).collect()
}

/// Probe each of `providers` once more, all at once, each under `cap`, and hand every
/// answer to `done` as it arrives (so a quick one is not held back by a slow one). It is
/// the whole policy: one retry, never a loop. `probe` is injected so tests need no CLI.
pub async fn retry_unknown<P, Fut>(providers: Vec<ProviderId>, cap: Duration, probe: P, mut done: impl FnMut(Availability))
where
    P: Fn(ProviderId) -> Fut,
    Fut: Future<Output = Availability> + Send + 'static,
{
    let mut running = tokio::task::JoinSet::new();
    for provider in providers {
        running.spawn(capped(cap, provider, probe(provider)));
    }
    while let Some(finished) = running.join_next().await {
        if let Ok(answer) = finished {
            done(answer);
        }
    }
}

/// Fold a late answer into `current`: it only fills in a provider that is still
/// [`State::Unknown`] (a `/providers refresh` may have found a definite answer since), and an
/// answer that is itself Unknown teaches nothing. True when something changed.
pub fn merge_late(current: &mut [Availability], late: Availability) -> bool {
    if late.state == State::Unknown {
        return false;
    }
    match current.iter_mut().find(|a| a.provider == late.provider && a.state == State::Unknown) {
        Some(slot) => {
            *slot = late;
            true
        }
        None => false,
    }
}

/// Fold a fresh answer into `current` (which came from the cache): it replaces the provider's
/// entry when it says something different. An Unknown answer teaches nothing and is dropped.
/// True when something changed.
pub fn merge_refresh(current: &mut [Availability], fresh: Availability) -> bool {
    if fresh.state == State::Unknown {
        return false;
    }
    match current.iter_mut().find(|a| a.provider == fresh.provider) {
        Some(slot) if *slot != fresh => {
            *slot = fresh;
            true
        }
        _ => false,
    }
}
