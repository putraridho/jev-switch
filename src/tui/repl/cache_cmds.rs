//! `/cache` and `/effort`: the cache dashboard and the reasoning-effort setting. Pure
//! wording lives in [`crate::turn::domain::cache`] and [`crate::turn::domain::effort`]; this maps them onto the session.

use crate::tui::repl::session::Repl;
use crate::backend::ProviderId;
use crate::turn::domain::cache;
use crate::catalog::domain::model_table as catalog;
use crate::turn::domain::effort;
use crate::turn::domain::effort::EffortSetting;
use crate::providers::application::provider_registry as providers;

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
