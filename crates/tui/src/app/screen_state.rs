//! The state of the interactive screen and what each key and each session message does to
//! it. No terminal here: [`App::on_key`] and [`App::on_front`] return [`Action`]s for the loop
//! in [`super::run`] to carry out, so every interaction is a unit test.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::oneshot;

use crate::input::editor::Editor;
use crate::input::history::History;
use crate::input::menu;
use crate::input::menu::{Menu, Then};
use crate::console::front::{ApprovalAsk, FrontMsg};
use crate::input::line_editor::approval::AnswerKey;
use crate::input::line_editor::interrupt::INTERRUPT_WINDOW;
use jev_approval::domain::mode::Mode;
use crate::render::CompleteCtx;
use crate::input::keymap::{Keymap, UserAction};
use crate::input::vim;
use crate::input::vim::{Outcome, Vim};

mod dialogs;
mod front;
mod keys;
mod completion;
#[cfg(test)]
mod tests;

/// What the loop must do after a key or a message.
#[derive(Debug)]
pub enum Action {
    /// Run this line now (the session is idle).
    Submit(String),
    /// Offer this queued message to the running turn.
    Steer(String),
    /// The person took this message back out of the queue to edit it: the turn must not
    /// send the copy it was offered.
    Unsteer(String),
    /// Stop the running turn.
    Stop,
    /// End the program.
    Quit,
    /// Put finished output above the live area.
    Print(String),
    /// The person picked another mode (Shift-Tab).
    Mode(Mode),
    ClearScreen,
    Bell,
    /// Give the terminal to a command; say so on the sender once it is free.
    Suspend(std_mpsc::Sender<()>),
    /// Take the terminal back.
    Resume,
}

/// A tool call waiting for the person: the card that replaces the input box.
#[derive(Debug)]
pub struct Approval {
    pub tool: String,
    pub notes: Vec<String>,
    pub lines: Vec<String>,
    /// What "don't ask again" saves; `None` when it is not on offer.
    pub always: Option<String>,
    pub note: Option<String>,
    reply: Option<oneshot::Sender<AnswerKey>>,
    pub selected: usize,
    /// First input row shown, when the input is longer than the card.
    pub scroll: usize,
    /// When the card appeared: keys in the first moments were typed before it could be read.
    shown_at: Instant,
}

/// Keys pressed this soon after a card appears were meant for something else (typing ahead);
/// they never answer it. Ctrl-C still stops the turn.
pub const CARD_GRACE: Duration = Duration::from_millis(250);

impl Approval {
    fn new(ask: ApprovalAsk, now: Instant) -> Self {
        Self { tool: ask.tool, notes: ask.notes, lines: ask.lines, always: ask.always, note: ask.note, reply: Some(ask.reply), selected: 0, scroll: 0, shown_at: now }
    }

    /// The choices, in order, with their answers.
    pub fn choices(&self) -> Vec<(AnswerKey, String)> {
        let mut v = vec![(AnswerKey::Yes, "Yes".to_string())];
        if let Some(always) = &self.always {
            v.push((AnswerKey::Always, format!("Yes, and don't ask again ({always})")));
        }
        v.push((AnswerKey::No, "No, and tell the model what to do instead".to_string()));
        v
    }

    /// The session dropped the question (the CLI withdrew the call).
    fn withdrawn(&self) -> bool {
        self.reply.as_ref().is_none_or(oneshot::Sender::is_closed)
    }

    fn answer(&mut self, key: AnswerKey) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(key);
        }
    }
}

/// A yes/no question from a command (`/undo`).
#[derive(Debug)]
pub struct Confirm {
    pub question: String,
    reply: Option<std_mpsc::Sender<bool>>,
    /// 0 is yes, 1 is no; no is the default, as `[y/N]` was.
    pub selected: usize,
}

impl Confirm {
    fn answer(&mut self, yes: bool) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(yes);
        }
    }
}

/// One open worker as the workers rows show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerChip {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub running: bool,
    /// How long ago it started.
    pub age_ms: u64,
    /// Tool calls made so far, and the latest one.
    pub tools: usize,
    pub activity: Option<String>,
}

/// The working row while a turn runs.
#[derive(Debug, Clone)]
pub struct Working {
    pub label: String,
    pub activity: Option<String>,
    pub started: Instant,
}

pub struct App {
    pub editor: Editor,
    history: History,
    pub menu: Option<Menu>,
    /// The menu was closed with Esc for this exact text; it opens again once the text changes.
    menu_closed_for: Option<String>,
    pub working: Option<Working>,
    pub approval: Option<Approval>,
    pub confirm: Option<Confirm>,
    /// Messages typed during a turn, run after it in order.
    pub queue: VecDeque<String>,
    pub mode: Mode,
    /// The pin or routing state, shown in the footer: `auto`, `codex`, `claude only`.
    pub route: String,
    pub ctx: CompleteCtx,
    project: PathBuf,
    /// The project folder as the header shows it: `~/Playground/jev-switch`.
    pub place: String,
    tracked: Vec<String>,
    last_ctrl_c: Option<Instant>,
    /// A short note in the footer (`Press Ctrl-C again to exit`) and when it was set.
    pub notice: Option<(String, Instant)>,
    /// The terminal belongs to a command right now.
    pub suspended: bool,
    /// Messages sent so far: the first one has no rule above it.
    pub sent: usize,
    /// The git branch and how many files differ from it: `main ±3`. `None` outside a repo.
    pub git: Option<String>,
    /// How full the model's context window was after the last turn, in percent.
    pub context: Option<u8>,
    /// The session's name (`/rename`).
    pub session_name: Option<String>,
    /// The subscriptions the router can use right now, for the router row.
    pub providers: Vec<ProviderChip>,
    /// A planner may hand pieces to workers (`/orchestrate`).
    pub orchestrate: bool,
    /// The workers the planner has open, oldest first, for the workers rows above the box.
    pub workers: Vec<WorkerChip>,
    /// Vim input mode (`JEV_VIM=1`); `None` when off.
    pub vim: Option<Vim>,
    /// The user's own key bindings.
    pub keymap: Keymap,
    /// What the last answer offered to do next, shown dim in the empty box; Right takes it.
    pub suggestion: Option<String>,
    /// The full-screen layout: the app keeps the conversation and draws the whole terminal.
    pub fullscreen: bool,
    /// The conversation, in the full-screen layout.
    pub transcript: crate::views::transcript::Transcript,
    /// Rows of the chat pane at the last draw, for how far a page scrolls.
    pub pane_rows: usize,
    /// The answer as it is being written, for the live preview in the full-screen layout.
    /// Never part of the conversation: the finished answer arrives as a `Print`.
    pub live_answer: Option<String>,
}

/// One subscription on the router row: its name, how much of its plan window is used, or how
/// long it is cooling down after a limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChip {
    pub name: String,
    pub used: Option<u8>,
    pub cooling: Option<String>,
    /// The CLI behind it when the name does not say (see `AvailRow::via`).
    pub via: Option<String>,
    /// Account credit left, for a pay-per-token provider: `$12.34 left`.
    pub credit: Option<String>,
}

/// How long a footer notice stays.
pub const NOTICE_FOR: Duration = Duration::from_secs(3);

fn ctrl(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL)
}

fn alt(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::ALT)
}

impl App {
    pub fn new(project: PathBuf, history: History, mode: Mode, route: String, tracked: Vec<String>) -> Self {
        let place = crate::views::status_facts::startup::display_path(&project, std::env::var("HOME").ok().as_deref());
        Self {
            place,
            editor: Editor::default(),
            history,
            menu: None,
            menu_closed_for: None,
            working: None,
            approval: None,
            confirm: None,
            queue: VecDeque::new(),
            mode,
            route,
            ctx: CompleteCtx::default(),
            project,
            tracked,
            last_ctrl_c: None,
            notice: None,
            suspended: false,
            sent: 0,
            git: None,
            context: None,
            session_name: None,
            providers: Vec::new(),
            orchestrate: false,
            workers: Vec::new(),
            vim: vim::enabled_from(std::env::var("JEV_VIM").ok().as_deref()).then(Vim::default),
            keymap: Keymap::default(),
            suggestion: None,
            fullscreen: false,
            transcript: crate::views::transcript::Transcript::default(),
            pane_rows: 20,
            live_answer: None,
        }
    }

    /// Scroll the chat pane: positive goes back in time. A no-op inline (the terminal's own
    /// scrollback does it).
    pub fn scroll_chat(&mut self, by: isize) {
        if self.fullscreen {
            self.transcript.scroll_by(by, self.pane_rows);
        }
    }

    /// Add finished output to the conversation (full-screen layout).
    pub fn show(&mut self, text: &str) {
        self.transcript.push_text(text);
    }

    /// Something to animate or expire: the loop redraws on its timer only then.
    pub fn needs_ticks(&self) -> bool {
        self.working.is_some() || self.notice.is_some() || self.workers.iter().any(|w| w.running)
    }

    /// Drop a notice that has been up long enough.
    pub fn expire(&mut self, now: Instant) {
        if self.notice.as_ref().is_some_and(|(_, at)| now.duration_since(*at) >= NOTICE_FOR) {
            self.notice = None;
        }
    }
}
