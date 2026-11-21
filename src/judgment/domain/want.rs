use crate::catalog::domain::model_table::TaskType;

use super::orchestration::Orchestration;
use super::security::Security;

/// Jev's signals and the fields used to choose a worker.
#[derive(Debug, Clone)]
pub struct JevSignals {
    raw: String,
    choice: Option<String>,
    confidence: Option<f64>,
    edit_p: Option<f64>,
}

impl JevSignals {
    pub fn new(raw: String, choice: Option<String>, confidence: Option<f64>, edit_p: Option<f64>) -> Self {
        Self { raw, choice, confidence, edit_p }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn is_null(&self) -> bool {
        self.raw == "null"
    }
}

impl Default for JevSignals {
    fn default() -> Self {
        Self::new("null".into(), None, None, None)
    }
}

#[derive(Debug)]
pub struct Decision {
    pub model: &'static str,
    pub kind: String,
    pub confidence: f64,
    pub reason: &'static str,
}

/// A code-mode routing judgment for one turn (see [`Router::code_want`]).
pub struct CodeWant {
    pub tier: usize,
    pub confident: bool,
    pub reason: String,
    /// Jev's raw signals for the usage log (null when Jev wasn't asked).
    pub signals: JevSignals,
}

/// Jev's reading of one turn for provider routing (see [`Router::provider_want`]).
/// Jev only reports; mapping a task type to models is the catalog's job.
#[derive(Debug, Clone)]
pub struct ProviderWant {
    pub task: TaskType,
    /// Jev's confidence in `task`.
    pub task_confidence: f64,
    /// P(only the most capable model will do).
    pub very_hard_p: f64,
    /// P(the request is about security), independent of keywords.
    pub security_p: f64,
    /// P(the request modifies files or runs commands).
    pub edit_p: f64,
    /// P(the message continues the task in progress rather than starting a new one).
    pub continues_p: f64,
    /// P(the message reacts to the previous answer as wrong); `None` with no previous turn.
    pub correction_p: Option<f64>,
    /// Jev's orchestration judgment: P(the request is a large task that splits into separate
    /// pieces), the signal orchestration triggers on (`ORCH_TRIGGER_P`), and the sequence
    /// veto. Asked in the same single batch.
    pub orchestration: Orchestration,
    /// What the keyword lists say, kept alongside Jev's judgment.
    pub security_keywords: Security,
    /// Every number above plus the task name, for the decision log.
    pub signals: JevSignals,
}

/// The model a worker should run on, as the alias `agent.spawn` takes. Fable is never a
/// worker. Opus may be one only when Fable is the planner (`allow_opus`), for a task Jev
/// judges security-sensitive or hard; under an Opus planner it stays on Sonnet and the
/// planner's own review checks it.
pub fn worker_alias(tier: usize, signals: &JevSignals, cheap: Option<f64>, allow_opus: bool) -> &'static str {
    if tier >= 2 {
        return if allow_opus { "opus" } else { "sonnet" };
    }
    let edit_p = signals.edit_p.unwrap_or(1.0);
    let confidence = signals.confidence.unwrap_or(0.0);
    match cheap {
        Some(c) if signals.choice.as_deref() == Some("simple") && confidence >= c && edit_p < 0.2 => "haiku",
        _ => "sonnet",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workers_follow_the_planner_haiku_only_for_read_only_lookups_when_enabled() {
        let lookup = json!({ "choice": "simple", "confidence": 0.95, "edit_p": 0.05 }).into();
        let edit = json!({ "choice": "simple", "confidence": 0.95, "edit_p": 0.7 }).into();
        assert_eq!(worker_alias(1, &lookup, None, false), "sonnet", "Haiku is off unless enabled");
        assert_eq!(worker_alias(1, &lookup, Some(0.9), false), "haiku");
        assert_eq!(worker_alias(1, &edit, Some(0.9), false), "sonnet", "anything that edits stays off Haiku");
        assert_eq!(worker_alias(1, &json!({ "choice": "simple", "confidence": 0.6, "edit_p": 0.05 }).into(), Some(0.9), false), "sonnet");
        // Opus only under a Fable planner, and only for security or hard tasks
        assert_eq!(worker_alias(2, &lookup, Some(0.9), false), "sonnet", "Opus planner: no Opus workers");
        assert_eq!(worker_alias(2, &lookup, Some(0.9), true), "opus", "Fable planner: Opus for hard or security tasks");
        assert_eq!(worker_alias(1, &edit, None, true), "sonnet", "an everyday task is still Sonnet");
        assert_eq!(worker_alias(3, &edit, None, false), "sonnet");
    }
}
