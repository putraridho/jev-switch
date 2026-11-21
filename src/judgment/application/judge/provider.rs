//! The whole provider-routing judgment: one request to Jev, one parsed [`ProviderWant`].

use anyhow::Result;
use serde_json::{json, Value};

use crate::catalog::domain::model_table::TaskType;
use crate::judgment::domain::orchestration::Orchestration;
use crate::judgment::domain::security::security_level;
use crate::judgment::domain::want::ProviderWant;
use crate::judgment::infrastructure::typesafe_http::{decode_choice, decode_noul};

use super::questions::*;
use super::Router;

impl Router {
    /// The whole provider-routing judgment in ONE request: which kind of task this is,
    /// plus the yes/no signals the router weighs it with. A single round trip (~0.5 s)
    /// instead of five keeps the added latency off every turn. `previous` is the prior
    /// (request, answer) pair; without it the correction question is not asked.
    /// `orchestrate`: orchestration is on, so the `sequential` veto question is asked too
    /// (it is used for nothing else). Bounded
    /// by `jev_timeout`; `None` on a timeout, any error or an unreadable answer, and the
    /// caller then stays on its current model.
    pub async fn provider_want(
        &self,
        request: &str,
        recent_context: &str,
        previous: Option<(&str, &str)>,
        orchestrate: bool,
    ) -> Option<ProviderWant> {
        let state = provider_state(request, recent_context, previous);
        let questions = provider_questions(previous.is_some(), orchestrate);
        let answers = tokio::time::timeout(self.jev_timeout, self.ts.ask_many(state, questions)).await.ok()?.ok()?;
        decode_provider_want(&answers, request, previous.is_some(), orchestrate).ok()
    }
}

/// Every question of a provider judgment as one `questions` map. The correction
/// question needs the previous exchange, so it is asked only when there is one; the
/// `sequential` veto only matters to orchestration, so it is asked only when that is on.
fn provider_questions(with_previous: bool, with_sequence: bool) -> Value {
    let mut q = json!({
        Q_TASK: task_type_question(),
        Q_CONTINUES: continues_question(),
        Q_EDIT: needs_changes_question(),
        Q_SECURITY: security_review_question(),
        Q_HARD: very_hard_question(),
        Q_MULTI_PART: multi_part_question(),
    });
    if with_previous {
        q[Q_CORRECTION] = is_correction_question();
    }
    if with_sequence {
        q[Q_SEQUENTIAL] = sequential_question();
    }
    q
}

/// The shared state for the batch: the usual request state, plus the previous exchange
/// when there is one (the correction question reads it).
fn provider_state(request: &str, recent_context: &str, previous: Option<(&str, &str)>) -> Value {
    let mut state = request_state(request, recent_context);
    if let Some((prev_request, prev_answer)) = previous {
        state["previous_request"] = json!(prev_request);
        state["previous_answer"] = json!(prev_answer);
    }
    state
}

/// Turn the batch's `answers` into a [`ProviderWant`]. An unknown task type name is an
/// error rather than a guess: the caller falls back to its current model. The `sequential`
/// answer is the exception: it only feeds the orchestration veto, so when it cannot be read
/// the judgment keeps every other signal, has no veto information, and the decision log
/// says why (`sequential_error`).
fn decode_provider_want(answers: &Value, request: &str, with_previous: bool, with_sequence: bool) -> Result<ProviderWant> {
    let task_answer = decode_choice(answers, Q_TASK)?;
    let task: TaskType = task_answer.choice.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let task = repo_question_task(request, task);
    let very_hard_p = decode_noul(answers, Q_HARD)?;
    let security_p = decode_noul(answers, Q_SECURITY)?;
    let edit_p = decode_noul(answers, Q_EDIT)?;
    let continues_p = decode_noul(answers, Q_CONTINUES)?;
    let sequential = with_sequence.then(|| decode_noul(answers, Q_SEQUENTIAL));
    let sequential_error = sequential.as_ref().and_then(|r| r.as_ref().err()).map(|e| format!("{e:#}"));
    let orchestration = Orchestration::judged(decode_noul(answers, Q_MULTI_PART)?, sequential.and_then(Result::ok));
    let correction_p = if with_previous { Some(decode_noul(answers, Q_CORRECTION)?) } else { None };
    let security_keywords = security_level(request);
    let mut signals = json!({
        "task": task.as_str(), "task_confidence": task_answer.confidence,
        "very_hard_p": very_hard_p, "security_p": security_p, "edit_p": edit_p,
        "continues_p": continues_p, "multi_part_p": orchestration.multi_part_p,
        "sequential_p": orchestration.sequential_p, "orch_vetoed": orchestration.vetoed, "correction_p": correction_p,
        "security": format!("{security_keywords:?}").to_lowercase(),
    });
    if let Some(e) = sequential_error {
        signals["sequential_error"] = json!(e);
    }
    Ok(ProviderWant {
        task,
        task_confidence: task_answer.confidence,
        very_hard_p,
        security_p,
        edit_p,
        continues_p,
        correction_p,
        orchestration,
        security_keywords,
        signals,
    })
}

/// Correct a current-info judgment for explicit questions about this local tool or repository.
pub(super) fn repo_question_task(request: &str, task: TaskType) -> TaskType {
    let lower = request.to_ascii_lowercase();
    let question = ["does ", "do ", "is ", "are ", "can ", "how ", "where ", "what "].iter().any(|prefix| lower.trim_start().starts_with(prefix));
    let local = ["jev-code", "this repo", "this codebase", "this repository"].iter().any(|s| lower.contains(s));
    let external = ["latest", "today", "news", "release notes"].iter().any(|s| lower.contains(s));
    if task == TaskType::CurrentInfo && question && local && !external { TaskType::Explain } else { task }
}
