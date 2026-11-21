//! The single-signal judgments: one yes/no question each.

use anyhow::Result;
use serde_json::json;

use crate::domain::orchestration::Orchestration;
use crate::application::wire::decode_noul;

use super::questions::*;
use super::Router;

impl Router {
    /// Code mode: probability that fulfilling `request` means modifying files or
    /// running commands (vs. only reading code and answering). Changing auth or
    /// session code goes to the strong model; reading it stays on mid.
    pub async fn needs_changes(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.noul_question(request_state(request, recent_context), needs_changes_question()).await
    }

    /// Code mode: probability that `request` is about security: who can access or
    /// change what, or whether something is safe against misuse. Catches authz and
    /// trust-boundary questions that carry no security keyword, and lets keyword
    /// matches that are about something else (design tokens, React keys) through.
    pub async fn security_review(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.noul_question(request_state(request, recent_context), security_review_question()).await
    }

    /// Probability that `request` is among the hardest work: it needs the most capable
    /// model, not just a strong one. Asked only of turns already headed to Opus.
    pub async fn very_hard(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.noul_question(request_state(request, recent_context), very_hard_question()).await
    }

    /// Orchestrate mode's judgment, from ONE Jev request asking both `multi_part` (is
    /// `request` a large task that splits into separate pieces of work, so a planner can
    /// hand them to cheaper workers; questions, single fixes and short follow-ups score
    /// low) and `sequential` (see [`SEQUENTIAL_VETO_P`]). The only way to ask
    /// `multi_part` for orchestration, so no caller can skip the sequence veto.
    ///
    /// An unreadable `sequential` answer does not fail the judgment: it only loses the veto
    /// (`sequential_p` is `None`), the same as in [`Router::provider_want`]; the caller logs it.
    pub async fn orchestration_p(&self, request: &str, recent_context: &str) -> Result<Orchestration> {
        let questions = json!({ Q_MULTI_PART: multi_part_question(), Q_SEQUENTIAL: sequential_question() });
        let answers = self.ts.ask_many(request_state(request, recent_context), questions).await?;
        Ok(Orchestration::judged(decode_noul(&answers, Q_MULTI_PART)?, decode_noul(&answers, Q_SEQUENTIAL).ok()))
    }

    /// Outcome signal: probability that `request` reacts to the previous answer as
    /// wrong, incomplete or not what was wanted (a correction, complaint, retry or
    /// the same question again). Logged for later analysis; never used for routing.
    pub async fn is_correction(&self, request: &str, previous_request: &str, previous_answer: &str) -> Result<f64> {
        let state = json!({ "request": request, "previous_request": previous_request, "previous_answer": previous_answer });
        self.noul_question(state, is_correction_question()).await
    }
}
