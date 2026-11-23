//! Circuit breakers: providers that keep failing are paused for a while, across runs.

use jev_backend::{ErrorKind, ProviderId};
use jev_shared::domain::duration_text::format_left;
use jev_shared::ALL_PROVIDERS;

use super::Runtime;

/// `<state_dir>/provider-health.json`, or `None` if there is no state directory.
pub fn health_path(host: &dyn crate::application::ports::ProviderHost) -> Option<std::path::PathBuf> {
    Some(host.state_dir()?.join("provider-health.json"))
}

/// What a failure says about the provider itself, or `None` when it says nothing the
/// breaker should hold against it: a limit (cooldowns cover it), a login (the user fixes it),
/// a refusal that depends on the folder or on one model, and failures nobody understands.
fn counted(kind: &ErrorKind) -> Option<&'static str> {
    match kind {
        ErrorKind::Empty => Some("empty answer"),
        ErrorKind::Stalled => Some("stalled"),
        ErrorKind::Unsupported => Some("retired CLI"),
        ErrorKind::Quota { .. } | ErrorKind::NotEntitled | ErrorKind::Auth | ErrorKind::UnusableHere | ErrorKind::Other => None,
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
    pub(super) fn note_failure(&mut self, p: ProviderId, kind: &ErrorKind, now_ms: u64) {
        let Some(what) = counted(kind) else { return };
        self.breakers.record_failure(p, what, now_ms);
        self.save_health();
    }

    /// A provider answered: whatever the breaker held against it is forgiven.
    pub fn note_success(&mut self, p: ProviderId, now_ms: u64) {
        if self.breakers.record_success(p, now_ms) {
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

    pub(super) fn pause_lines(&self, now_ms: u64) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| {
                let b = self.breakers.open(*p, now_ms)?;
                Some(format!(
                    "{p} paused for {} after {} failures in a row (last: {}); /providers refresh tries it now",
                    format_left(b.open_until_ms - now_ms),
                    b.failures,
                    b.last
                ))
            })
            .collect()
    }
}
