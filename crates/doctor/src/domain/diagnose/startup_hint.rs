//! The short startup nudge.

use jev_providers::domain::availability::{Availability, State};
use jev_providers::domain::login_hints::DEFAULT_ENV_PATH;

use super::provider_checks::fix_steps;

/// A short nudge for startup, or `None` when everything useful is ready. Shown when
/// fewer than two providers are ready (routing has nothing to choose between) or one
/// is installed but not logged in. Providers that are not installed are never
/// mentioned individually.
pub fn first_run_hint(avail: &[Availability]) -> Option<String> {
    let ready = avail.iter().filter(|a| a.state == State::Ready).count();
    let half_configured: Vec<&Availability> = avail
        .iter()
        .filter(|a| a.state == State::NotLoggedIn || (ready < 2 && a.state == State::NoKey))
        .collect();
    if ready >= 2 && half_configured.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    match ready {
        0 => lines.push("No provider is ready yet, so nothing can run.".to_string()),
        1 => lines.push("Only one provider is ready, so there is nothing to route between.".to_string()),
        _ => {}
    }
    for a in half_configured {
        let first = fix_steps(a.provider, a.state, DEFAULT_ENV_PATH).into_iter().next().unwrap_or_default();
        let why = if a.state == State::NoKey { "has no API key" } else { "is installed but not logged in" };
        lines.push(format!("{} {why}: {first}", a.provider));
    }
    if ready == 0 && lines.len() == 1 {
        lines.push("Run doctor for the install steps.".to_string());
    }
    Some(lines.join("\n"))
}
