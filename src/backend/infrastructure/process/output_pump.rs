//! Reading a CLI's stdout to its end and acting on what it finds.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::sync::mpsc;

use super::stdin_feed::StdinFeed;
use crate::approval::domain::rule_policy::{Decision, PermissionRequest};
use crate::backend::application::ports::{Approver, Backend, Steering, NO_ONE_TO_ASK};
use crate::backend::domain::event::{Event, Finish};
use crate::backend::domain::plan_usage::PlanUsage;

/// How long the CLI may stay silent after its answer before the runner stops reading and
/// reaps it (see [`Pump::next_line`]).
const AFTER_ANSWER_WAIT: Duration = Duration::from_secs(2);

/// How the stdout stream ended.
pub(crate) enum Ended {
    Finished(Finish),
    Failed(String),
    Eof,
}

/// Marker for "no stdout line within the idle timeout".
pub(crate) struct Idle;

/// Reads the CLI's stdout to its end and acts on what it finds.
///
/// Outside a question the read has the idle watchdog: `idle` of silence is a stall, unless
/// the answer already came. During a question (a [`PermissionRequest`] being decided) stdout
/// is read too, with no watchdog, so a [`Event::PermissionWithdrawn`], the result, or the CLI
/// dying reach the runner while the person is still deciding. Events that arrive then are
/// held and delivered, in order, once the question is over.
pub(crate) struct Pump<'a> {
    pub(super) backend: &'a dyn Backend,
    pub(super) reader: BufReader<ChildStdout>,
    /// The line being read. Kept until it is complete: a read that lost a `select!` race
    /// leaves its partial bytes here and the next read continues them.
    pub(super) raw: Vec<u8>,
    pub(super) idle: Duration,
    pub(super) on_event: &'a mut (dyn FnMut(&Event) + Send),
    pub(super) plan: &'a mut Option<PlanUsage>,
    pub(super) approver: &'a dyn Approver,
    pub(super) feed: &'a mut StdinFeed,
    pub(super) finished: Option<Finish>,
    /// Questions the CLI asked while another was being decided, oldest first.
    pub(super) queue: VecDeque<PermissionRequest>,
    /// Messages from the person while the turn runs, to be added to it ([`Backend::steer_line`]).
    pub(super) steer: Option<Steering<'a>>,
    /// Messages written to the CLI that it has not reported picking up yet, oldest first: what
    /// the person typed, and what was sent of it.
    pub(crate) sent: VecDeque<(String, String)>,
    /// Tools denied because nobody could be asked ([`NO_ONE_TO_ASK`]), in order: an empty
    /// answer after one is that denial, not a model failure (see [`judge`]).
    pub(crate) denied_unasked: Vec<String>,
}

/// The next message the person typed, waiting forever when there is no sender or it is gone.
async fn next_steer(steer: Option<&mut mpsc::UnboundedReceiver<String>>) -> String {
    match steer {
        Some(rx) => match rx.recv().await {
            Some(text) => text,
            // The sender is gone: nothing more will come.
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

/// What a line did to the wait it arrived in.
enum Signal {
    Failed(String),
    Finished,
    Withdrawn(String),
}

/// How the read of one line went.
enum Line {
    Events(Vec<Event>),
    Eof,
    Idle,
}

/// What a question did to the stream.
enum Asked {
    /// Over, answered or withdrawn: read on.
    Over,
    /// The stream ended or failed while it was waiting.
    Ended(Ended),
}

impl Pump<'_> {
    /// The line the last read completed, as events; `Eof` at the end of the stream.
    fn take_line(&mut self, read: std::io::Result<usize>) -> Line {
        if !matches!(read, Ok(n) if n > 0) && self.raw.is_empty() {
            return Line::Eof;
        }
        let events = self.backend.parse_line(&String::from_utf8_lossy(&self.raw));
        for response in self.backend.take_responses() {
            self.feed.send(response);
        }
        self.raw.clear();
        Line::Events(events)
    }

    /// The next line, under the idle watchdog. Once the answer is in, the CLI only has to
    /// exit: a short wait, not the full watchdog, so something it left running (a background
    /// shell, a monitor) cannot hold the session for minutes after the answer.
    async fn next_line(&mut self) -> Line {
        let wait = if self.finished.is_some() { self.idle.min(AFTER_ANSWER_WAIT) } else { self.idle };
        // One deadline for the line: a message typed meanwhile is not a sign of life from the CLI.
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let Self { reader, raw, steer, .. } = &mut *self;
            let steer = steer.as_mut().map(|s| &mut s.rx);
            // Writing a message to the CLI does not wait for the next line, so a message typed
            // while a long tool runs goes in at once.
            let typed = tokio::select! {
                read = tokio::time::timeout_at(deadline, reader.read_until(b'\n', raw)) => match read {
                    Ok(read) => return self.take_line(read),
                    Err(_) => return Line::Idle,
                },
                typed = next_steer(steer) => typed,
            };
            self.steer_in(typed);
        }
    }

    /// Add a message the person typed to the running turn. Not once the answer is in: stdin is
    /// closing then, and the message stays with the person's queue to run as the next turn.
    fn steer_in(&mut self, text: String) {
        if self.finished.is_some() {
            return;
        }
        let Some(prepared) = self.steer.as_ref().and_then(|s| (s.prepare)(&text)) else { return };
        if let Some(line) = self.backend.steer_line(&prepared) {
            self.feed.send(format!("{line}\n"));
            self.sent.push_back((text, prepared));
        }
    }

    fn deliver(&mut self, event: Event) {
        if let Event::Plan(p) = &event {
            *self.plan = Some(p.clone());
        }
        (self.on_event)(&event);
    }

    /// Apply one event. Something to show goes to `held` while a question is pending and
    /// to `on_event` at once otherwise; what changes the wait comes back as a [`Signal`].
    fn absorb(&mut self, event: Event, held: Option<&mut Vec<Event>>) -> Option<Signal> {
        match event {
            Event::Finished(f) => {
                self.finished = Some(f);
                self.queue.clear();
                // An interactive CLI keeps reading stdin until told there is no more.
                self.feed.close();
                Some(Signal::Finished)
            }
            Event::Failed { message } => Some(Signal::Failed(message)),
            Event::PermissionRequest(req) => {
                if self.finished.is_none() {
                    self.queue.push_back(req);
                }
                None
            }
            Event::PermissionWithdrawn { id } => {
                self.queue.retain(|q| q.id != id);
                Some(Signal::Withdrawn(id))
            }
            // The CLI echoes the person's own prompt too: only what was sent in is reported.
            Event::Steered(text) => {
                // Oldest first, but a message the CLI never echoes must not hide the later ones.
                if let Some(at) = self.sent.iter().position(|(_, sent)| sent.trim() == text.trim()) {
                    if let Some((typed, _)) = self.sent.remove(at) {
                        match held {
                            Some(held) => held.push(Event::Steered(typed)),
                            None => self.deliver(Event::Steered(typed)),
                        }
                    }
                }
                None
            }
            shown => {
                match held {
                    Some(held) => held.push(shown),
                    None => self.deliver(shown),
                }
                None
            }
        }
    }

    /// Decide `req` while reading on. Its answer is written once, when the decision
    /// completes. A withdrawal of `req`, the result, a failure or the end of the stream
    /// drops the decision instead and tells the approver; nothing is written then.
    async fn ask(&mut self, req: PermissionRequest) -> Asked {
        let approver = self.approver;
        let mut decision = approver.decide(&req);
        let mut held: Vec<Event> = Vec::new();
        let mut ended: Option<Ended> = None;
        let answer = loop {
            let Self { reader, raw, steer, .. } = &mut *self;
            let steer = steer.as_mut().map(|s| &mut s.rx);
            // A message typed while a question waits goes to the CLI as well: it has it at its next step.
            let line = tokio::select! {
                d = &mut decision => break Some(d),
                read = reader.read_until(b'\n', raw) => self.take_line(read),
                typed = next_steer(steer) => {
                    self.steer_in(typed);
                    continue;
                }
            };
            let Line::Events(events) = line else {
                ended = Some(Ended::Eof);
                break None;
            };
            let mut over = false;
            for event in events {
                match self.absorb(event, Some(&mut held)) {
                    Some(Signal::Failed(m)) => {
                        ended.get_or_insert(Ended::Failed(m));
                        over = true;
                    }
                    Some(Signal::Finished) => over = true,
                    Some(Signal::Withdrawn(id)) => over |= id == req.id,
                    None => {}
                }
            }
            if over {
                break None;
            }
        };
        match answer {
            Some(d) => {
                if matches!(&d, Decision::Deny { reason } if reason == NO_ONE_TO_ASK) {
                    self.denied_unasked.push(req.tool.clone());
                }
                if let Some(line) = self.backend.permission_response(&req, &d) {
                    self.feed.send(format!("{line}\n"));
                }
            }
            None => {
                drop(decision);
                approver.withdrawn(&req);
            }
        }
        for event in held {
            self.deliver(event);
        }
        ended.map_or(Asked::Over, Asked::Ended)
    }

    /// The answer is in while a message the CLI has not taken sits in its pipe: stop reading now,
    /// so the CLI is not given the time to start on it (see `run_child`).
    fn answer_left_a_message(&self) -> bool {
        self.finished.is_some() && !self.sent.is_empty()
    }

    /// The result of the stream so far, once it is over.
    fn conclude(&mut self) -> Ended {
        self.finished.take().map_or(Ended::Eof, Ended::Finished)
    }

    pub(super) async fn run(&mut self) -> Result<Ended, Idle> {
        'read: loop {
            let events = match self.next_line().await {
                Line::Events(events) => events,
                Line::Eof => break,
                Line::Idle if self.finished.is_some() => break,
                Line::Idle => return Err(Idle),
            };
            for event in events {
                if let Some(Signal::Failed(message)) = self.absorb(event, None) {
                    return Ok(Ended::Failed(message));
                }
            }
            if self.answer_left_a_message() {
                break;
            }
            while let Some(req) = self.queue.pop_front() {
                match self.ask(req).await {
                    // The answer may have come while the question was open.
                    Asked::Over if self.answer_left_a_message() => break 'read,
                    Asked::Over => {}
                    Asked::Ended(Ended::Failed(m)) => return Ok(Ended::Failed(m)),
                    Asked::Ended(_) => return Ok(self.conclude()),
                }
            }
        }
        Ok(self.conclude())
    }
}
