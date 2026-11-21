//! Plan utilization and the DeepSeek balance: what each provider reports about how much
//! of its allowance is used.

use crate::backend::{PlanUsage, ProviderId};

use super::Runtime;

impl Runtime {
    /// Codex reports its limits only in its session files, not in the turn's output.
    pub(super) fn refresh_codex_plan(&mut self) {
        if let Some((plan, _)) = crate::backend::infrastructure::codex::codex_plan::latest_plan() {
            self.note_plan(ProviderId::Codex, plan);
        }
    }

    /// Fetch DeepSeek's balance off the prompt's path when it has a key. The answer lands
    /// in `balance` whenever it arrives; until then (or on any failure) none is shown.
    pub(super) fn refresh_balance(&self) {
        let Some(key) = crate::shared::infrastructure::env_file::lookup("DEEPSEEK_API_KEY").filter(|k| !k.trim().is_empty()) else { return };
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let slot = self.balance.clone();
        handle.spawn(async move {
            if let Some(b) = crate::providers::infrastructure::deepseek_balance::fetch_deepseek_balance(&key).await {
                *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(b);
            }
        });
    }

    /// Update what a finished turn may have changed: Codex's plan reading, DeepSeek's balance.
    pub fn after_turn(&mut self, provider: ProviderId) {
        match provider {
            ProviderId::Codex => self.refresh_codex_plan(),
            ProviderId::DeepSeek => self.refresh_balance(),
            _ => {}
        }
    }

    /// DeepSeek's remaining balance for display, `$12.34 left`, once the fetch has answered.
    pub fn credit_left(&self, p: ProviderId) -> Option<String> {
        if p != ProviderId::DeepSeek {
            return None;
        }
        self.balance.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(crate::providers::infrastructure::deepseek_balance::format_balance)
    }

    pub fn note_plan(&mut self, p: ProviderId, plan: PlanUsage) {
        if self.plan.get(&p) == Some(&plan) {
            return;
        }
        self.plan.insert(p, plan);
        if let Some(path) = &self.plan_path {
            crate::providers::infrastructure::plan_store::save(path, &self.plan);
        }
    }

    /// Fraction of the plan window used (as the CLI reported it), if a reading exists and
    /// its window has not reset since.
    pub fn plan_utilization(&self, p: ProviderId) -> Option<f64> {
        let now_s = crate::shared::infrastructure::jsonl_log::now_ms() / 1000;
        self.plan.get(&p).filter(|u| crate::providers::infrastructure::plan_store::is_current(u, now_s)).map(|u| u.utilization)
    }
}
