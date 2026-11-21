//! The task type that picks the chain: Jev's label, or the hard security rule over it.

use super::Signals;
use crate::catalog::domain::model_table::TaskType;
use crate::judgment::application::judge::{CODE_EDIT_P, CODE_SECURITY_P, Security};

/// Where the task type came from, which decides how it is described and how much
/// weight it carries when deciding to switch provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskSource {
    Jev,
    /// The hard security rule overrode Jev's task type.
    SecurityRule,
    NoSignals,
}

/// The task type that picks the chain. Security is a HARD rule that overrides Jev:
/// a confident-looking wrong answer is costly there and Jev's task label alone does
/// not reliably catch it. Without Jev there is no judgment, so the task is Unclear.
pub(super) fn effective_task(signals: Option<&Signals>) -> (TaskType, TaskSource) {
    let Some(s) = signals else { return (TaskType::Unclear, TaskSource::NoSignals) };
    let security_rule = s.security_keywords == Security::Strong
        || s.security_p >= CODE_SECURITY_P
        || (s.security_keywords == Security::Weak && s.edit_p >= CODE_EDIT_P);
    if security_rule && s.task != TaskType::Security {
        (TaskType::Security, TaskSource::SecurityRule)
    } else {
        (s.task, TaskSource::Jev)
    }
}

pub(super) fn task_label(task: TaskType, source: TaskSource, signals: Option<&Signals>) -> String {
    match (source, signals) {
        (TaskSource::Jev, Some(s)) => format!("{task} ({:.2})", s.task_confidence),
        (TaskSource::SecurityRule, _) => format!("{task} (security rule)"),
        _ => format!("{task} (Jev unavailable)"),
    }
}
