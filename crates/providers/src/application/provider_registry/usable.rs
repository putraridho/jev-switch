//! What routing may pick right now, and the cooldowns and refusals behind it.

use std::borrow::Cow;
use std::sync::PoisonError;

use jev_backend::ProviderId;
use jev_catalog::domain::model_table::Usable;
use crate::domain::availability::Availability;
use crate::domain::reset_time::cooldowns::Cooldowns;
use crate::domain::usability::usable_from;

use super::{BlockReason, Runtime};

impl Runtime {
    /// This process's cooldowns plus whatever other processes saved to the file. The
    /// file is only re-read when it changed (one `stat` otherwise); with no file this
    /// is just the in-memory set.
    pub(super) fn current_cooldowns(&self) -> Cow<'_, Cooldowns> {
        let Some(path) = &self.cooldown_path else { return Cow::Borrowed(&self.cooldowns) };
        let mut disk = self.disk.lock().unwrap_or_else(PoisonError::into_inner);
        Cow::Owned(self.cooldowns.union(disk.refresh(path, &*self.host)))
    }

    /// The (provider, model) pairs routing may pick right now.
    pub fn usable(&self, now_ms: u64) -> Usable {
        let cooldowns = self.current_cooldowns();
        let skipped = |p: ProviderId| self.blocked.contains_key(&p) || self.breakers.open(p, now_ms).is_some();
        if !self.avail.iter().any(|a| skipped(a.provider)) {
            return usable_from(&self.avail, &cooldowns, &self.unusable_models, now_ms).with_windows(self.windows.clone());
        }
        let unblocked: Vec<Availability> = self.avail.iter().filter(|a| !skipped(a.provider)).cloned().collect();
        usable_from(&unblocked, &cooldowns, &self.unusable_models, now_ms).with_windows(self.windows.clone())
    }

    /// The tokens a session may fill for `spec`: the window its backend reported, else the catalog's.
    pub fn window_of(&self, spec: &jev_catalog::domain::model_table::ModelSpec) -> u64 {
        self.windows.get(&(spec.provider, spec.id.to_string())).copied().unwrap_or(spec.context_tokens)
    }

    /// How long `p` is still cooling down after a usage limit, in ms; `None` when it is not.
    pub fn cooldown_left(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.current_cooldowns().active(p, now_ms)
    }

    /// Skip `p` until its limit resets, and remember that across runs and processes:
    /// the save merges with the file, and the merged result (which includes limits
    /// other processes saved) becomes this process's set. A failed save costs only the
    /// persistence, so it is reported and otherwise ignored.
    pub fn cool_down(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.cooldowns.start_in(p, resets_at, now_ms, &|text, zone| self.host.resolve_reset_time(text, zone), &|zone| self.host.reset_zone_exists(zone));
        let Some(path) = &self.cooldown_path else { return };
        match self.host.save_cooldowns(path, &self.cooldowns, now_ms) {
            Ok(merged) => self.cooldowns = merged,
            Err(e) => eprintln!("could not save cooldowns to {}: {e}", path.display()),
        }
    }

    /// Stop offering `model` of `p` for the rest of this run.
    pub fn mark_unusable(&mut self, p: ProviderId, model: &str) {
        self.unusable_models.insert((p, model.to_string()));
    }

    /// `/new`: the directory-dependent blocks end with the session they were learned in.
    pub fn forget_unusable_here(&mut self) {
        self.blocked.retain(|_, reason| *reason != BlockReason::UnusableHere);
    }
}
