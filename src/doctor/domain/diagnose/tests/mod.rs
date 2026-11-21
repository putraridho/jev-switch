//! Tests for [`super`], grouped by the check family they cover. Shared fixtures live here.

use super::*;
use crate::providers::domain::availability::{Availability, State};
use crate::shared::ProviderId;
use crate::shared::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

mod hint;
mod jev;
mod machine;
mod providers;
mod report;
mod state;

pub(super) fn av(provider: ProviderId, state: State) -> Availability {
    let verified = matches!(provider, Claude | Codex | DeepSeek);
    Availability { provider, state, plan: None, models: None, detail: String::new(), verified }
}

pub(super) fn all(states: [State; 5]) -> Vec<Availability> {
    [Claude, DeepSeek, Codex, Gemini, Grok].into_iter().zip(states).map(|(p, s)| av(p, s)).collect()
}

pub(super) fn inputs(avail: Vec<Availability>) -> Inputs {
    Inputs { avail, jev_key_present: true, jev_reachable: Some(true), ..Inputs::default() }
}

pub(super) fn find<'a>(r: &'a Report, id: &str) -> &'a Check {
    r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}"))
}

pub(super) fn has_check(r: &Report, id: &str) -> bool {
    r.checks.iter().any(|c| c.id == id)
}
