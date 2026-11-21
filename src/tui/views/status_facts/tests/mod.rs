//! Tests of the status facts by subject, with the shared fixtures here.

use crate::backend::ProviderId;
use crate::backend::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};
use crate::providers::application::provider_registry::Runtime;
use crate::providers::domain::availability::{Availability, State};
use crate::turn::application::engine::Engine;
use std::path::PathBuf;

mod doctor;
mod pager;
mod prompt;
mod providers;
mod startup;
mod turns;

pub(super) const NOW: u64 = 1_760_000_000_000;

pub(super) fn av(provider: ProviderId, state: State) -> Availability {
    Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
}

pub(super) fn runtime() -> Runtime {
    Runtime::for_tests(vec![
        av(Claude, State::Ready),
        av(DeepSeek, State::NoKey),
        av(Codex, State::Ready),
        av(Gemini, State::NotInstalled),
        av(Grok, State::NotLoggedIn),
    ])
}

pub(super) fn engine(rt: Runtime) -> Engine {
    use crate::judgment::application::judge::Router;
    use crate::judgment::infrastructure::typesafe_http::TypeSafe;
    Engine::new(rt, Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), PathBuf::from("/home/me/proj"), crate::turn::infrastructure::engine_adapters::real())
        .with_turn_log(None)
}
