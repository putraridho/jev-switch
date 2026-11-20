//! What a backend finds in one line of a CLI's output.

use super::plan_usage::PlanUsage;
use crate::approval::domain::rule_policy::PermissionRequest;
use crate::shared::Usage;

/// The terminal success event of a CLI's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finish {
    pub text: String,
    pub session_id: Option<String>,
    pub usage: Usage,
}

/// One thing a backend found in one stdout line.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// One-line summary of a tool call, like "Bash cargo test".
    Tool(String),
    /// The model's reasoning before it acts (extended thinking), never part of the answer.
    Thinking(String),
    /// What a tool call returned, as the CLI reported it (the text may be long; the screen
    /// shows a line of it). `ok` is false when the tool failed.
    ToolResult { ok: bool, text: String },
    /// The change an edit tool is about to make: the file, the lines it takes out and the
    /// lines it puts in (a whole new file has nothing taken out).
    Edit { path: String, removed: Vec<String>, added: Vec<String> },
    /// Assistant text: part of the answer.
    Text(String),
    /// A piece of assistant text as it is written, in order. For the screen only: the whole
    /// text still arrives once as [`Event::Text`] (or in [`Finish::text`]).
    TextDelta(String),
    /// A diagnostic about the turn that is never part of the answer: a tool call that was
    /// denied or failed, with what to do about it. Shown on stderr as it happens.
    Hint(String),
    Finished(Finish),
    Failed { message: String },
    Plan(PlanUsage),
    /// The CLI is waiting for the host to say whether it may run a tool. Only backends
    /// that are [`Backend::interactive`] produce it; [`run_turn_with`] answers it through
    /// the [`Approver`] and it never reaches `on_event`.
    PermissionRequest(PermissionRequest),
    /// The CLI no longer needs the answer to the request `id` (the turn was interrupted, a
    /// hook settled it, it timed out). [`run_turn_with`] drops the pending question, tells the
    /// [`Approver`] ([`Approver::withdrawn`]) and writes no answer for it; it never reaches
    /// `on_event`. An id that is not waiting (answered already, or not a question) is ignored.
    PermissionWithdrawn { id: String },
    /// A message the person typed while the turn ran (see [`Backend::steer_line`]) was picked
    /// up by the model: it is part of this turn now. The text is the message as sent.
    /// Backends report every user message they echo; [`run_turn_steered`] passes on only the
    /// ones it sent, and the person's own prompt is never one of them.
    Steered(String),
}
