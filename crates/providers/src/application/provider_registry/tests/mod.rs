//! Tests of [`Runtime`], by subject, with the helpers they share.

mod cooldowns;
mod late_probes;
mod plan;
mod react;
mod render;
mod usable;

use jev_backend::ProviderId;
use jev_catalog::domain::model_table as catalog;
use crate::domain::availability::{Availability, State};
use ProviderId::{Claude, Codex, DeepSeek, Gemini};

use super::Runtime;

const NOW: u64 = 1_760_000_000_000;
const MINUTE_MS: u64 = 60_000;

    pub(super) fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    pub(super) fn some_model(p: ProviderId) -> &'static catalog::ModelSpec {
        catalog::models().iter().find(|m| m.provider == p).expect("catalog has the provider")
    }

    pub(super) fn runtime() -> Runtime {
        Runtime::for_tests(vec![
            availability(Claude, State::Ready),
            availability(Codex, State::Ready),
            availability(Gemini, State::NotLoggedIn),
            availability(DeepSeek, State::NoKey),
        ])
    }
