//! What a turn needs from the outside: a CLI to run ([`Backend`]), someone to ask about
//! tools ([`Approver`]), a way to build a backend per provider, and messages typed mid-turn.

use std::future::Future;
use std::pin::Pin;

use tokio::process::Command;
use tokio::sync::mpsc;

use jev_approval::domain::permission_request::{Decision, PermissionRequest};
use crate::domain::effort::Effort;
use crate::domain::elicitation::{ElicitationRequest, ElicitationAnswer};
use crate::domain::error_kind::ErrorKind;
use crate::domain::event::Event;
use crate::domain::mcp_attachment::McpAttachment;
use crate::domain::turn_request::TurnRequest;
use jev_shared::ProviderId;

/// Why a request is denied when nobody can be asked.
pub const NO_ONE_TO_ASK: &str = "no one to ask: this session has no terminal to approve tool calls";

/// Whoever answers tool permissions and MCP elicitation while a turn runs. The turn waits for the answer
/// without any idle timeout: a person can take as long as they need. The turn keeps reading
/// the CLI meanwhile, so the decision may be abandoned: the future is dropped and
/// [`Approver::withdrawn`] follows. Only a future that yields can be abandoned, so an
/// implementation that waits for a person must wait asynchronously.
pub trait Approver: Send + Sync {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>>;

    /// The question for `req` was asked and then dropped without an answer: the CLI withdrew
    /// it, finished, or died. Take the question off the screen. Called after the `decide`
    /// future is dropped, once per such question, never for one that was answered.
    fn withdrawn(&self, _req: &PermissionRequest) {}

    /// Ask the person directly. Permission modes and remembered tool rules do not answer it.
    fn elicit<'a>(&'a self, _req: &'a ElicitationRequest) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        Box::pin(std::future::ready(ElicitationAnswer::Cancel))
    }

    fn elicitation_withdrawn(&self, _req: &ElicitationRequest) {}
}

/// The approver of a run that has no one to ask: every request is denied at once.
pub struct NoApprover;

impl Approver for NoApprover {
    fn decide<'a>(&'a self, _: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async { Decision::Deny { reason: NO_ONE_TO_ASK.to_string() } })
    }
}

/// What one CLI contributes. Object-safe and synchronous on purpose: the only
/// async part (process IO) is the same for every CLI and lives in [`run_turn`].
pub trait Backend: Send + Sync {
    fn provider(&self) -> ProviderId;

    /// Fully configured command (program, args, env, `current_dir = req.dir`).
    /// Stdin, stdout and stderr are NOT set; the runner pipes all three.
    fn command(&self, req: &TurnRequest) -> Command;

    /// Text written to the child's stdin, which is then closed. `None` when the
    /// prompt travels in the args instead.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        Some(req.prompt.clone())
    }

    /// Parse ONE stdout line. Non-JSON or irrelevant lines give an empty Vec.
    fn parse_line(&self, line: &str) -> Vec<Event>;

    /// Whether this CLI asks the host before running a tool ([`Event::PermissionRequest`])
    /// and waits for [`Backend::permission_response`] on stdin. Such a CLI keeps stdin open
    /// until it has answered the turn; every other CLI gets its prompt and EOF at once.
    fn interactive(&self) -> bool {
        false
    }

    /// The line (without the newline) that answers `req` on the CLI's stdin. Only called
    /// when [`Backend::interactive`].
    fn permission_response(&self, _req: &PermissionRequest, _decision: &Decision) -> Option<String> {
        None
    }

    /// Reply to a server's question on the provider control channel.
    fn elicitation_response(&self, _req: &ElicitationRequest, _answer: &ElicitationAnswer) -> Option<String> {
        None
    }

    /// The line (without the newline) that adds `text` as a message from the person to the turn
    /// that is running, or `None` when this CLI cannot take one mid-turn. Only a backend that is
    /// [`Backend::interactive`] can: its stdin stays open. The CLI answers with
    /// [`Event::Steered`] once the model has the message, which is at the next step of the
    /// turn, not at once.
    fn steer_line(&self, _text: &str) -> Option<String> {
        None
    }

    /// Whether [`Backend::steer_line`] works for this backend as configured.
    fn steerable(&self) -> bool {
        false
    }

    /// Protocol messages produced while parsing stdout (for example ACP setup replies).
    /// Each includes its framing, and is sent before reading the next stdout line.
    fn take_responses(&self) -> Vec<String> {
        Vec::new()
    }

    /// Whether [`Backend::attach_mcp`] works for this CLI. Orchestration only picks a
    /// planner among providers that say yes: a planner that cannot call the delegate
    /// tool could not delegate anything. Gemini and Grok say no in this version.
    fn supports_mcp(&self) -> bool {
        false
    }

    /// Add `mcp` to this backend's commands. Only called when [`Backend::supports_mcp`].
    fn attach_mcp(&mut self, _mcp: &McpAttachment) {}

    /// Set the reasoning effort of this backend's turns. A CLI with no such control
    /// ignores it (see [`jev_turn::domain::effort::supported`]).
    fn set_effort(&mut self, _effort: Effort) {}

    /// Ask this CLI for partial assistant text, when it can give it. A backend with no
    /// partial stream ignores it; the answer still arrives whole.
    fn set_streaming(&mut self, _on: bool) {}

    /// Classify an error message from the CLI (a `Failed` text or a stderr tail).
    /// Implementations call [`classify_common`] first and may add their own rules.
    fn classify_error(&self, message: &str) -> ErrorKind;

    /// Called by [`run_turn`] once the turn is over, whatever its result: remove
    /// anything [`Backend::command`] left on disk (see [`PromptFiles`]).
    fn cleanup(&self) {}
}

pub type BackendFactory = Box<dyn Fn(ProviderId) -> Box<dyn Backend> + Send + Sync>;

/// Messages the person types while a turn runs, and how one is made fit to send.
pub struct Steering<'a> {
    pub rx: mpsc::UnboundedReceiver<String>,
    /// The text as it may be sent (secrets blanked), or `None` to keep it out of this turn.
    pub prepare: &'a (dyn Fn(&str) -> Option<String> + Send + Sync),
}
