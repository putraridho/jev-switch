//! Plan utilization and the DeepSeek balance: what each provider reports about how much
//! of its allowance is used.

use jev_backend::{PlanUsage, ProviderId};

use super::Runtime;

/// The least time between two Gemini quota questions: each one starts `agy`.
const GEMINI_PLAN_EVERY_MS: u64 = 5 * 60 * 1000;

/// What the background Gemini quota fetch has found, and when it was last started.
#[derive(Debug, Default)]
pub(super) struct GeminiPlanSlot {
    pub(super) reading: std::sync::Arc<std::sync::Mutex<Option<PlanUsage>>>,
    asked_ms: Option<u64>,
}

impl GeminiPlanSlot {
    fn reading(&self) -> Option<PlanUsage> {
        self.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

impl Runtime {
    /// Codex reports its limits only in its session files, not in the turn's output.
    pub(super) fn refresh_codex_plan(&mut self) {
        if let Some(plan) = self.host.latest_codex_plan() {
            self.note_plan(ProviderId::Codex, plan);
        }
    }

    /// Ask for Gemini's quota off the prompt's path, at most every few minutes. On by
    /// default; `JEV_GEMINI_USAGE=off` turns it off. The answer is used whenever it
    /// arrives; until then (or on any failure) the saved reading stands.
    pub(super) fn refresh_gemini_plan(&mut self) {
        if self.host.env("JEV_GEMINI_USAGE").is_some_and(|v| v.trim().eq_ignore_ascii_case("off")) {
            return;
        }
        let now = self.host.now_ms();
        if self.gemini_plan.asked_ms.is_some_and(|asked| now.saturating_sub(asked) < GEMINI_PLAN_EVERY_MS) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        self.gemini_plan.asked_ms = Some(now);
        let slot = self.gemini_plan.reading.clone();
        let host = self.host.clone();
        handle.spawn(async move {
            if let Some(plan) = host.fetch_gemini_plan().await {
                *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(plan);
            }
        });
    }

    /// Fetch DeepSeek's balance off the prompt's path when it has a key. The answer lands
    /// in `balance` whenever it arrives; until then (or on any failure) none is shown.
    pub(super) fn refresh_balance(&self) {
        let Some(key) = self.host.env("DEEPSEEK_API_KEY").filter(|k| !k.trim().is_empty()) else { return };
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let slot = self.balance.clone();
        let host = self.host.clone();
        handle.spawn(async move {
            if let Some(b) = host.fetch_credit(key).await {
                *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(b);
            }
        });
    }

    /// Update what a finished turn may have changed: Codex's and Gemini's plan readings,
    /// DeepSeek's balance.
    pub fn after_turn(&mut self, provider: ProviderId) {
        match provider {
            ProviderId::Codex => self.refresh_codex_plan(),
            ProviderId::DeepSeek => self.refresh_balance(),
            ProviderId::Gemini => {
                // Keep (and save) what an earlier fetch found, then ask again.
                if let Some(plan) = self.gemini_plan.reading() {
                    self.note_plan(ProviderId::Gemini, plan);
                }
                self.refresh_gemini_plan();
            }
            _ => {}
        }
    }

    /// DeepSeek's remaining balance for display, `$12.34 left`, once the fetch has answered.
    pub fn credit_left(&self, p: ProviderId) -> Option<String> {
        if p != ProviderId::DeepSeek {
            return None;
        }
        self.balance.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn note_plan(&mut self, p: ProviderId, plan: PlanUsage) {
        if self.plan.get(&p) == Some(&plan) {
            return;
        }
        self.plan.insert(p, plan);
        if let Some(path) = &self.plan_path {
            self.host.save_plans(path, &self.plan);
        }
    }

    /// Fraction of the plan window used (as the CLI reported it), if a reading exists and
    /// its window has not reset since.
    pub fn plan_utilization(&self, p: ProviderId) -> Option<f64> {
        let now_s = self.host.now_ms() / 1000;
        // A fresh background answer beats the saved reading before a turn has stored it.
        let fresh = if p == ProviderId::Gemini { self.gemini_plan.reading() } else { None };
        fresh.as_ref().or_else(|| self.plan.get(&p)).filter(|u| self.host.plan_is_current(u, now_s)).map(|u| u.utilization)
    }
}
