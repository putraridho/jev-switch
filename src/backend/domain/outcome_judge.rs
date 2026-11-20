//! Deciding whether a finished stream is an answer or a failure.

use super::{error_kind::ErrorKind, error_text::resets_tail, event::Finish, plan_usage::PlanUsage, turn_error::TurnError, turn_outcome::TurnOutcome};
use crate::shared::domain::limit_notice::is_limit_message;

/// An answer can still be a failure: empty, or the Claude CLI's way of saying the
/// plan is used up (normal answer text, no error flag).
pub(crate) fn judge(finish: Finish, plan: Option<PlanUsage>) -> Result<TurnOutcome, TurnError> {
    if finish.text.trim().is_empty() {
        return Err(TurnError::new(ErrorKind::Empty, "the CLI returned an empty answer"));
    }
    if is_limit_message(&finish.text) {
        let resets_at = resets_tail(&finish.text);
        return Err(TurnError::new(
            ErrorKind::Quota { resets_at },
            format!("usage limit reached: {}", finish.text.trim()),
        ));
    }
    Ok(TurnOutcome { text: finish.text, session_id: finish.session_id, usage: finish.usage, plan })
}

/// An empty answer after a tool was denied because nobody could be asked is that denial, however
/// the CLI reports it (an empty result, or its own "empty response" failure): every model would
/// hit it, so it is not [`ErrorKind::Empty`] (which tries the next model) but a failure that ends
/// the turn and says how to allow the tool.
pub(crate) fn empty_after_denial(err: TurnError, denied_unasked: &[String]) -> TurnError {
    if err.kind == ErrorKind::Empty && !denied_unasked.is_empty() {
        return TurnError::new(ErrorKind::Other, denied_unasked_message(denied_unasked));
    }
    err
}

/// Why a turn ended empty after denials nobody could answer, and how to allow the tools.
pub(crate) fn denied_unasked_message(tools: &[String]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for t in tools {
        if !names.contains(&t.as_str()) {
            names.push(t);
        }
    }
    format!(
        "stopped without an answer after {} was denied: nobody could be asked in this run; allow it with CODE_ALLOWED_TOOLS (e.g. \"Bash(python3 *)\"), or run in an interactive session",
        names.join(", ")
    )
}
