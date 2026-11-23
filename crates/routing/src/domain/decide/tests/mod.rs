//! Tests for [`super`], grouped by subject. Shared fixtures live here.

use super::*;
use jev_catalog::domain::model_table as catalog;
use jev_catalog::domain::model_table::{Billing, allowed_for, default_chain};
use jev_judgment::domain::thresholds::MAX_HOLD;
use super::switch_gate::SHORT_FOLLOW_UP_WORDS;
use jev_shared::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

mod evidence;
mod holding;
mod pins;
mod properties;
mod switching;
mod task_rules;

pub(super) const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

pub(super) fn sig(task: TaskType) -> Signals {
    Signals {
        task,
        task_confidence: 0.9,
        very_hard_p: 0.0,
        security_p: 0.0,
        edit_p: 0.0,
        continues_p: 0.0,
        correction_p: None,
        orchestration: Orchestration::default(),
        security_keywords: Security::None,
    }
}

/// A cache the provider reported reading `tokens` of for `model`, ended at time 0.
pub(super) fn warm_of(provider: ProviderId, model: &str, tokens: u64) -> WarmCache {
    let usage = jev_shared::Usage {
        input_tokens: tokens,
        cached_input_tokens: tokens,
        output_tokens: 1,
        ..Default::default()
    };
    WarmCache::after_turn(
        &crate::domain::cache_facts::CacheKey::new(provider, model, "b"),
        &usage,
        0,
        None
    )
}

pub(super) struct Fixture {
    pub(super) usable: Usable,
    pub(super) allow: Allowances,
    pub(super) overrides: Overrides,
    pub(super) pin: Pin,
    pub(super) context_tokens: u64,
    pub(super) plan: Option<f64>,
    pub(super) now_ms: u64,
    pub(super) records: Option<crate::domain::track_record::TrackRecords>,
}

impl Fixture {
    pub(super) fn new(providers: &[ProviderId]) -> Self {
        Self {
            usable: Usable::all_of(providers.iter().copied()),
            allow: Allowances { credits: true, per_token: true },
            overrides: Overrides::default(),
            pin: Pin::Auto,
            context_tokens: 0,
            plan: None,
            now_ms: 0,
            records: None,
        }
    }

    pub(super) fn run(&self, state: &mut RoutingState, signals: Option<&Signals>, words: usize) -> Result<RouteDecision, NoRoute> {
        let plan = self.plan;
        let util = move |p: ProviderId| if p == Claude { plan } else { None };
        decide(
            state,
            &RouteInput {
                signals,
                prompt_words: words,
                pin: &self.pin,
                usable: &self.usable,
                allow: &self.allow,
                overrides: &self.overrides,
                context_tokens: self.context_tokens,
                plan_utilization: &util,
                now_ms: self.now_ms,
                records: self.records.as_ref(),
            },
        )
    }
}

pub(super) fn on(id: &str) -> RoutingState {
    let spec = models().iter().find(|m| m.id == id).unwrap();
    let mut s = RoutingState::default();
    record_turn(&mut s, spec);
    s
}

pub(super) const NEW_TASK_WORDS: usize = 20;
