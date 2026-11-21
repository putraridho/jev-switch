//! Whether a turn may change provider at all.

use super::RouteInput;
use super::task::TaskSource;
use crate::catalog::domain::model_table::TaskType;

/// Jev says a new task is a clear provider-switch candidate only from this
/// `task_confidence`; below it the turn is too vague to justify a cold session.
const MIN_BOUNDARY_CONFIDENCE: f64 = 0.5;
/// A turn "continues" the previous work when `continues_p` reaches this; below it the
/// turn is a new task.
const CONTINUES_P: f64 = 0.5;
/// At or below this many words a prompt is a short, context-dependent follow-up
/// ("go on", "why?"). Mirrors [`crate::judgment::application::judge::is_short_follow_up`], which takes text
/// rather than the word count this pure module is handed; a test keeps them equal.
pub(super) const SHORT_FOLLOW_UP_WORDS: usize = 8;
/// Below this task confidence a boundary keeps the current provider when it already
/// has an equally strong model: not worth a cold session on a shaky judgment.
const CONFIDENT_SWITCH: f64 = 0.7;
/// A correction this likely means the current provider may itself be the problem.
const CORRECTION_P: f64 = 0.7;


/// Why a provider switch is allowed this turn.
pub(super) struct Gate {
    /// Strong enough evidence that the "keep the provider if it is as good" shortcut
    /// does not apply.
    pub(super) confident: bool,
    pub(super) why: &'static str,
    /// A rule (security, a likely correction) wants the switch whatever the cache holds.
    pub(super) forced: bool,
}

/// Whether the turn may change provider. A provider switch costs a cold session plus
/// a handoff, so it needs one of: a confident new task (not a short follow-up), a
/// likely correction (the current provider may be the problem), or Security work
/// (the hard rule must reach the models that are allowed for it).
pub(super) fn switch_gate(input: &RouteInput, task: TaskType, source: TaskSource) -> Option<Gate> {
    let s = input.signals?;
    if task == TaskType::Security {
        return Some(Gate { confident: source == TaskSource::SecurityRule || s.task_confidence >= CONFIDENT_SWITCH, why: "security work", forced: true });
    }
    if s.correction_p.is_some_and(|p| p >= CORRECTION_P) {
        return Some(Gate { confident: true, why: "likely correction, current provider may be the problem", forced: true });
    }
    let new_task = s.continues_p < CONTINUES_P
        && input.prompt_words > SHORT_FOLLOW_UP_WORDS
        && s.task_confidence >= MIN_BOUNDARY_CONFIDENCE;
    new_task.then_some(Gate { confident: s.task_confidence >= CONFIDENT_SWITCH, why: "new task, confident", forced: false })
}
