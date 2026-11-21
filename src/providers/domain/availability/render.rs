//! The lines `/providers` prints.

use super::{Availability, State};

/// The lines `/providers` prints, one per provider.
pub fn render(avail: &[Availability]) -> String {
    avail.iter().map(render_line).collect::<Vec<_>>().join("\n")
}

/// Only the providers that are ready, for the compact startup summary.
pub fn render_ready(avail: &[Availability]) -> String {
    avail.iter().filter(|a| a.state == State::Ready).map(render_line).collect::<Vec<_>>().join("\n")
}

fn render_line(a: &Availability) -> String {
    let plan = a.plan.as_deref().unwrap_or("-");
    let models = match (&a.state, &a.models) {
        (State::Ready, Some(list)) => list.join(", "),
        (State::Ready, None) => "all".to_string(),
        _ => "-".to_string(),
    };
    let unverified = if a.verified { "" } else { " (unverified)" };
    format!(
        "{:<9} {:<13} plan: {}  models: {}  {}{}",
        a.provider.as_str(),
        a.state.label(),
        plan,
        models,
        a.detail,
        unverified
    )
}
