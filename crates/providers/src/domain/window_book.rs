//! The context windows the machine's own backends reported, kept per (provider, model, backend).
//!
//! A window belongs to the backend that served it: a custom endpoint's window for a model must
//! never be applied to the same model on its native login (and the reverse), so the backend's
//! identity ([`crate::domain::funding::Fundings::identity`]) is part of the key. Reported values
//! are bounded: a zero or absurd number is dropped rather than trusted. The catalog's own size
//! stays the fallback for a model nothing reported.

use std::collections::HashMap;

use jev_shared::ProviderId;

/// The smallest window taken as real (a window under this is a misreading, not a model).
pub const MIN_WINDOW_TOKENS: u64 = 1_000;
/// The largest window taken as real; no model comes near it, so a bigger number is garbage.
pub const MAX_WINDOW_TOKENS: u64 = 64_000_000;

/// `tokens` when it is a believable window.
pub fn sane(tokens: u64) -> Option<u64> {
    (MIN_WINDOW_TOKENS..=MAX_WINDOW_TOKENS).contains(&tokens).then_some(tokens)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowBook(HashMap<(ProviderId, String, String), u64>);

impl WindowBook {
    /// Remember `tokens` for the model on that backend; `false` (and nothing kept) when the
    /// number is not believable.
    pub fn record(
        &mut self,
        provider: ProviderId,
        model: &str,
        backend: &str,
        tokens: u64
    ) -> bool {
        let Some(tokens) = sane(tokens) else {
            return false;
        };
        self.0.insert((provider, model.to_string(), backend.to_string()), tokens);
        true
    }

    /// Take every entry of `other`; where both have one, `other`'s is newer.
    pub fn absorb(&mut self, other: WindowBook) {
        self.0.extend(other.0);
    }

    pub fn get(&self, provider: ProviderId, model: &str, backend: &str) -> Option<u64> {
        self.0.get(&(provider, model.to_string(), backend.to_string())).copied()
    }

    /// The windows that hold for the backends in force now: `backend_of` names each provider's
    /// effective backend, and an entry learned on another one is left out.
    pub fn in_force(
        &self,
        backend_of: impl Fn(ProviderId) -> String
    ) -> HashMap<(ProviderId, String), u64> {
        self.0
            .iter()
            .filter(|((provider, _, backend), _)| *backend == backend_of(*provider))
            .map(|((provider, model, _), tokens)| ((*provider, model.clone()), *tokens))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{ Claude, Codex };

    #[test]
    fn zero_and_absurd_windows_are_rejected() {
        let mut book = WindowBook::default();
        for bad in [0, 1, 999, MAX_WINDOW_TOKENS + 1, u64::MAX] {
            assert!(!book.record(Claude, "m", "Claude login", bad), "{bad}");
        }
        assert!(book.is_empty());
        assert!(book.record(Claude, "m", "Claude login", 1_000_000));
        assert!(book.record(Claude, "edge", "Claude login", MIN_WINDOW_TOKENS));
        assert!(book.record(Claude, "edge2", "Claude login", MAX_WINDOW_TOKENS));
    }

    #[test]
    fn a_custom_endpoints_window_is_never_applied_to_the_native_login() {
        let mut book = WindowBook::default();
        book.record(Codex, "gpt", "custom endpoint (billing unknown)", 128_000);
        book.record(Codex, "gpt", "ChatGPT login", 900_000);
        let native = book.in_force(|_| "ChatGPT login".to_string());
        assert_eq!(native[&(Codex, "gpt".to_string())], 900_000);
        let custom = book.in_force(|_| "custom endpoint (billing unknown)".to_string());
        assert_eq!(custom[&(Codex, "gpt".to_string())], 128_000);
        assert!(
            book.in_force(|_| "API key (per token)".to_string()).is_empty(),
            "learned on neither"
        );
        assert_eq!(book.get(Codex, "gpt", "ChatGPT login"), Some(900_000));
    }

    #[test]
    fn a_newer_report_replaces_an_older_one_for_the_same_key() {
        let mut book = WindowBook::default();
        book.record(Claude, "m", "b", 200_000);
        let mut newer = WindowBook::default();
        newer.record(Claude, "m", "b", 1_000_000);
        book.absorb(newer);
        assert_eq!(book.get(Claude, "m", "b"), Some(1_000_000));
    }
}
