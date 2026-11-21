//! The answer to "may this prompt go there?" and the words for refusing.

use std::collections::BTreeMap;

use super::{locality, Locality};
use jev_shared::ProviderId;

/// The outcome of [`Policy::check`] for one prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    local_only: bool,
    denials: BTreeMap<ProviderId, String>,
}

impl Verdict {
    pub(super) fn new(local_only: bool, denials: BTreeMap<ProviderId, String>) -> Self {
        Self { local_only, denials }
    }

    /// Nothing is restricted.
    pub fn open() -> Self {
        Self { local_only: false, denials: BTreeMap::new() }
    }

    /// Whether the policy restricts this prompt at all (local-only, or a rule that fired).
    pub fn restricts_any(&self) -> bool {
        self.local_only || !self.denials.is_empty()
    }

    /// Why `provider` may not get this prompt, or `None` when it may.
    pub fn denial(&self, provider: ProviderId) -> Option<String> {
        if self.local_only && locality(provider) == Locality::Cloud {
            return Some("local-only mode is on".into());
        }
        self.denials.get(&provider).cloned()
    }

    /// Add `other`'s reason for refusing `provider` (if it has one and this verdict does not):
    /// for text only that provider will get, such as its catch-up.
    pub fn take_denial(&mut self, provider: ProviderId, other: &Verdict) {
        if let Some(why) = other.denials.get(&provider) {
            self.denials.entry(provider).or_insert_with(|| why.clone());
        }
    }

    pub fn permits(&self, provider: ProviderId) -> bool {
        self.denial(provider).is_none()
    }

    /// One line for the person when every provider is refused: the reasons, merged.
    pub fn refusal_line(&self) -> String {
        if self.local_only {
            return "local-only mode is on and no local provider is set up (see /privacy)".into();
        }
        let mut reasons: Vec<&String> = self.denials.values().collect();
        reasons.dedup();
        match reasons.first() {
            Some(r) => format!("privacy policy: {r}"),
            None => "privacy policy refuses every provider".into(),
        }
    }
}
