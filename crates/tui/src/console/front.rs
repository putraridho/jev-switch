//! The channel from the session (the [`crate::console::terminal_ui::Console`] and the commands that use it)
//! to the interactive front end ([`crate`]). With a front attached the console stops
//! drawing anything itself: finished lines, the turn's status and every question become
//! typed messages, and the front decides how they look and reads the keys that answer them.
//!
//! The session runs on its own task while the front keeps reading keys, so a question is a
//! message carrying the sender of its answer.

use std::sync::mpsc as std_mpsc;

use tokio::sync::{mpsc, oneshot};

use crate::input::line_editor::approval::AnswerKey;

/// A tool approval waiting for its answer.
#[derive(Debug)]
pub struct ApprovalAsk {
    /// The tool's name: `Bash`, `Edit`, ...
    pub tool: String,
    /// Facts about the call that are not part of its input (shown apart from it).
    pub notes: Vec<String>,
    /// The input, one entry per line, never cut.
    pub lines: Vec<String>,
    /// What "always" would save, in words; `None` when "always" is not on offer.
    pub always: Option<String>,
    /// Why "always" is not on offer, or what else it allows; shown under the choices.
    pub note: Option<String>,
    /// Where the answer goes. Dropped by the session when the CLI withdraws the call.
    pub reply: oneshot::Sender<AnswerKey>,
}

/// One thing the session tells the front.
#[derive(Debug)]
pub enum FrontMsg {
    /// Finished output, styled, possibly several lines: it goes above the live area, into
    /// the scrollback, and never changes again.
    Print(String),
    /// More assistant text while the answer is being written. Transient: it is drawn in
    /// place and never enters the conversation; the finished answer arrives as a `Print`.
    AnswerDelta(String),
    /// Drop the answer being written: it was narration, a failed attempt, or the finished
    /// answer has just been printed.
    AnswerReset,
    /// A turn started: show the working row.
    TurnStarted,
    /// The turn is over: remove the working row.
    TurnEnded,
    /// The model the turn runs on, `provider · model`.
    Label(String),
    /// What the model is doing right now (the last tool call, "stopping...").
    Activity(String),
    /// A tool call waits for the person.
    Approval(Box<ApprovalAsk>),
    /// The pending approval was withdrawn by the CLI: take it off the screen.
    ApprovalGone,
    /// A yes/no question from a command (`/undo`); the answer goes back on `reply`.
    Confirm { question: String, reply: std_mpsc::Sender<bool> },
    /// A command needs the real terminal (an editor, a pager, a login): give it back and say
    /// so on `ack`; [`FrontMsg::Resume`] follows when the command is done.
    Suspend { ack: std_mpsc::Sender<()> },
    /// The terminal is the front's again.
    Resume,
    /// Clear the screen and the scrollback.
    ClearScreen,
    /// Ring the bell (a long turn is over).
    Bell,
    /// The model took this queued message into the running turn: it no longer waits.
    Steered(String),
}

/// The session's end of the channel. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Front(mpsc::UnboundedSender<FrontMsg>);

impl Front {
    /// A front and the receiver the interactive loop reads.
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<FrontMsg>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(tx), rx)
    }

    /// Tell the front; a front that is gone (the program is exiting) hears nothing.
    pub fn send(&self, msg: FrontMsg) {
        let _ = self.0.send(msg);
    }

    /// Ask a yes/no question and wait for the answer. Blocking: the session calls it from its
    /// own task (told to the runtime as blocking), while the front keeps running. A front
    /// that is gone answers no.
    pub fn confirm(&self, question: &str) -> bool {
        let (reply, answer) = std_mpsc::channel();
        self.send(FrontMsg::Confirm { question: question.to_string(), reply });
        block(|| answer.recv().unwrap_or(false))
    }

    /// Run `f` with the real terminal (the front steps aside first and comes back after).
    pub fn with_terminal<T>(&self, f: impl FnOnce() -> T) -> T {
        let (ack, ready) = std_mpsc::channel();
        self.send(FrontMsg::Suspend { ack });
        // A front that is gone cannot be in the way.
        block(|| {
            let _ = ready.recv();
        });
        let out = f();
        self.send(FrontMsg::Resume);
        out
    }
}

/// Run a blocking wait from code that may be on the async runtime.
fn block<T>(f: impl FnOnce() -> T) -> T {
    let multi_thread = tokio::runtime::Handle::try_current().is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if multi_thread {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_confirm_waits_for_the_front_and_a_gone_front_says_no() {
        let (front, mut rx) = Front::channel();
        let answering = tokio::spawn(async move {
            match rx.recv().await {
                Some(FrontMsg::Confirm { question, reply }) => {
                    assert_eq!(question, "Revert 2 files?");
                    let _ = reply.send(true);
                }
                other => panic!("expected a confirm, got {other:?}"),
            }
        });
        assert!(front.confirm("Revert 2 files?"));
        answering.await.expect("the answering task ran");

        let (gone, rx) = Front::channel();
        drop(rx);
        assert!(!gone.confirm("anything?"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_terminal_waits_for_the_front_to_step_aside_and_resumes_after() {
        let (front, mut rx) = Front::channel();
        let steps = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(msg) = rx.recv().await {
                match msg {
                    FrontMsg::Suspend { ack } => {
                        seen.push("suspend");
                        let _ = ack.send(());
                    }
                    FrontMsg::Resume => {
                        seen.push("resume");
                        break;
                    }
                    _ => {}
                }
            }
            seen
        });
        assert_eq!(front.with_terminal(|| 7), 7);
        assert_eq!(steps.await.expect("ran"), vec!["suspend", "resume"]);
    }
}
