use std::collections::HashSet;

use super::models::models;
use super::ModelSpec;
use jev_shared::ProviderId;

/// The (provider, model id) pairs the user can use. Built by the caller from
/// logins, keys and entitlements; this module does not discover them.
#[derive(Debug, Default, Clone)]
pub struct Usable {
    set: HashSet<(ProviderId, String)>,
}

impl Usable {
    pub fn new<S: AsRef<str>>(iter: impl IntoIterator<Item = (ProviderId, S)>) -> Self {
        Self { set: iter.into_iter().map(|(p, id)| (p, id.as_ref().to_string())).collect() }
    }

    /// Every catalog model of the given providers, for callers that know only
    /// which providers are logged in.
    pub fn all_of(providers: impl IntoIterator<Item = ProviderId>) -> Self {
        let providers: Vec<_> = providers.into_iter().collect();
        Self::new(models().iter().filter(|m| providers.contains(&m.provider)).map(|m| (m.provider, m.id)))
    }

    pub fn contains(&self, spec: &ModelSpec) -> bool {
        self.set.contains(&(spec.provider, spec.id.to_string()))
    }

    /// The models for which `keep(provider, model id)` is true (policy, budget and user rules).
    pub fn filter(&self, keep: impl Fn(ProviderId, &str) -> bool) -> Self {
        Self { set: self.set.iter().filter(|(p, id)| keep(*p, id)).cloned().collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Whether any model of `provider` is in the set.
    pub fn has_provider(&self, provider: ProviderId) -> bool {
        self.set.iter().any(|(p, _)| *p == provider)
    }

    /// Only the models of `keep` (for orchestration: a planner must come from providers
    /// that can have the delegate tool attached).
    pub fn only_providers(&self, keep: &[ProviderId]) -> Self {
        Self { set: self.set.iter().filter(|(p, _)| keep.contains(p)).cloned().collect() }
    }

    /// The same set minus `excluded` (for one turn's failover: models that just failed).
    pub fn without(&self, excluded: &HashSet<(ProviderId, String)>) -> Self {
        Self { set: self.set.difference(excluded).cloned().collect() }
    }
}

