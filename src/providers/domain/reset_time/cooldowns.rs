//! Which providers are cooling down and until when, kept in memory and in a shared
//! JSON file.

use std::collections::HashMap;
use super::{cooldown_ms_in, Resolver, ZoneExists};
use crate::shared::ProviderId;

/// When each provider's usage limit resets (epoch ms).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cooldowns {
    until_ms: HashMap<ProviderId, u64>,
}

impl Cooldowns {
    /// Start (or restart) a cooldown using the supplied wall-clock facts.
    pub fn start_in(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) {
        self.until_ms.insert(p, now_ms.saturating_add(cooldown_ms_in(resets_at, now_ms, resolve, zone_exists)));
    }

    /// Milliseconds left if the provider is cooling down, else `None`.
    pub fn active(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.until_ms.get(&p).and_then(|u| u.checked_sub(now_ms)).filter(|left| *left > 0)
    }

    /// Forget a cooldown in this copy only. It does NOT lift a saved cooldown: once a
    /// cooldown has been saved it cannot be forgiven, it can only expire. That is on
    /// purpose. Saving merges the file in (later expiry per provider
    /// wins) and [`crate::providers::application::provider_registry::Runtime`] unions the file into what it reports,
    /// because other processes share the file and one process must not erase a limit
    /// another just learned. So after `clear` the entry comes back from disk on the
    /// next save or read. Nothing in production calls this; it exists for tests and
    /// for dropping a throwaway copy.
    pub fn clear(&mut self, p: ProviderId) {
        self.until_ms.remove(&p);
    }

    /// Per provider, the later of the two expiries (expired entries kept).
    pub fn union(&self, other: &Self) -> Self {
        let mut until_ms = self.until_ms.clone();
        for (p, u) in &other.until_ms {
            let e = until_ms.entry(*p).or_insert(*u);
            *e = (*e).max(*u);
        }
        Self { until_ms }
    }

    /// The later expiry per provider across both sets, minus entries already over.
    pub fn merged(&self, other: &Self, now_ms: u64) -> Self {
        let mut m = self.union(other);
        m.until_ms.retain(|_, u| *u > now_ms);
        m
    }

    pub fn entries(&self) -> &HashMap<ProviderId, u64> {
        &self.until_ms
    }
}

impl From<HashMap<ProviderId, u64>> for Cooldowns {
    fn from(until_ms: HashMap<ProviderId, u64>) -> Self {
        Self { until_ms }
    }
}
