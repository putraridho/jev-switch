//! Circuit breakers: providers that keep failing are paused for a while, across runs.

use jev_backend::{ErrorKind, ProviderId};
use crate::domain::breaker::Admission;
use crate::domain::readiness::{classify, RetryPolicy};
use jev_shared::domain::duration_text::format_left;
use jev_shared::ALL_PROVIDERS;

use super::Runtime;

/// `<state_dir>/provider-health.json`, or `None` if there is no state directory.
pub fn health_path(host: &dyn crate::application::ports::ProviderHost) -> Option<std::path::PathBuf> {
    Some(host.state_dir()?.join("provider-health.json"))
}

/// What a transient failure says about the provider, or `None` when the breaker is not the
/// one to hold it: a limit (cooldowns cover it), and lasting failures (a login, a retired
/// client, a folder, a model), which hold the provider until the fact changes (see
/// [`crate::domain::readiness`]).
fn counted(kind: &ErrorKind) -> Option<&'static str> {
    classify(kind).filter(|c| c.policy == RetryPolicy::Breaker)?;
    match kind {
        ErrorKind::Stalled => Some("stalled"),
        _ => Some("empty answer"),
    }
}

impl Runtime {
    /// Read the saved breakers; called once the host is known.
    pub(super) fn load_health(&mut self) {
        self.breaker_path = health_path(&*self.host);
        if let Some(path) = &self.breaker_path {
            self.breakers = self.host.load_breakers(path);
        }
    }

    /// Count a failed attempt against the provider when it says something about the provider.
    pub(super) fn note_failure(&mut self, p: ProviderId, model: &str, kind: &ErrorKind, now_ms: u64) {
        self.note_readiness_failure(p, model, kind, now_ms);
        let Some(what) = counted(kind) else { return };
        self.breakers.record_failure(p, what, now_ms);
        self.save_health();
    }

    /// A provider answered: whatever the breaker or a hold had against it is forgiven, and the
    /// ladder is proven up to this project.
    pub fn note_success(&mut self, p: ProviderId, now_ms: u64) {
        if self.breakers.record_success(p, now_ms) {
            self.save_health();
        }
        self.note_readiness_success(p, now_ms);
    }

    /// An attempt on `p` is about to start: after a pause, the first one is the trial that
    /// decides, and the others wait for it.
    pub fn begin_attempt(&mut self, p: ProviderId, now_ms: u64) {
        if self.breakers.begin_trial(p, now_ms) {
            self.save_health();
        }
    }

    /// `/providers refresh`: the user asks to try everything again.
    pub(super) fn forget_pauses(&mut self, now_ms: u64) {
        if self.breakers.clear(now_ms) {
            self.save_health();
        }
    }

    /// Save, merging with what other processes saved; a failed save costs only the persistence.
    fn save_health(&mut self) {
        let Some(path) = &self.breaker_path else { return };
        match self.host.save_breakers(path, &self.breakers) {
            Ok(merged) => self.breakers = merged,
            Err(e) => eprintln!("could not save provider health to {}: {e}", path.display()),
        }
    }

    /// How long `p` stays paused by its breaker, in ms; `None` when it is not paused.
    pub fn paused_left(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.breakers.open(p, now_ms).map(|b| b.open_until_ms - now_ms)
    }

    /// Whether the breaker keeps `p` out of routing right now.
    pub(super) fn breaker_blocks(&self, p: ProviderId, now_ms: u64) -> bool {
        self.breakers.admission(p, now_ms).blocks()
    }

    pub(super) fn pause_lines(&self, now_ms: u64) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| {
                let b = self.breakers.entries().get(p)?;
                let seen = format!("{} failures in a row (last: {})", b.failures, b.last);
                match self.breakers.admission(*p, now_ms) {
                    Admission::Closed => None,
                    Admission::Open { until_ms } => Some(format!(
                        "{p} paused for {} after {seen}; /providers refresh tries it now",
                        format_left(until_ms - now_ms)
                    )),
                    Admission::HalfOpen => Some(format!("{p} pause ended after {seen}: the next attempt is a trial; an answer closes it")),
                    Admission::Trial { .. } => Some(format!("{p} is being tried after a pause ({seen})")),
                }
            })
            .collect()
    }
}
