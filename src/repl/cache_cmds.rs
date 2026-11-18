//! `/cache` and `/effort`: the cache dashboard and the reasoning-effort setting. Pure
//! wording lives in [`crate::cache`] and [`crate::effort`]; this maps them onto the session.

use super::Repl;
use crate::backend::ProviderId;
use crate::cache;
use crate::catalog;
use crate::effort::{self, EffortSetting};
use crate::providers;

impl Repl {
    /// `/cache`.
    pub(super) fn cache_text(&self) -> String {
        cache::dashboard(&self.log, self.engine.warm_cache().as_ref(), self.now(), self.out_style())
    }

    /// Providers with at least one model that can run now.
    fn ready_providers(&self) -> Vec<ProviderId> {
        let usable = self.engine.runtime.usable(self.now());
        providers::ALL_PROVIDERS.into_iter().filter(|p| catalog::models().iter().any(|m| m.provider == *p && usable.contains(m))).collect()
    }

    /// `/effort` with no argument shows the setting; with one, sets it.
    pub(super) fn effort(&mut self, setting: Option<EffortSetting>) {
        if let Some(s) = setting {
            self.engine.set_effort(s);
        }
        let text = effort::effort_text(self.engine.effort(), &self.ready_providers());
        self.page_out(&text);
    }
}
