//! The state of the interactive screen and what each key and each session message does to
//! it. No terminal here: [`App::on_key`] and [`App::on_front`] return [`Action`]s for the loop
//! in [`super::run`] to carry out, so every interaction is a unit test.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::oneshot;

use crate::tui::input::editor::Editor;
use crate::tui::input::history::History;
use crate::tui::input::menu;
use crate::tui::input::menu::{Menu, Then};
use crate::tui::console::front::{ApprovalAsk, FrontMsg};
use crate::tui::input::line_editor::{AnswerKey, INTERRUPT_WINDOW};
use crate::approval::domain::mode::Mode;
use crate::tui::render::CompleteCtx;
use crate::tui::input::keymap::{Keymap, UserAction};
use crate::tui::input::vim;
use crate::tui::input::vim::{Outcome, Vim};

/// What the loop must do after a key or a message.
#[derive(Debug)]
pub enum Action {
    /// Run this line now (the session is idle).
    Submit(String),
    /// Offer this queued message to the running turn.
    Steer(String),
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
    pub transcript: crate::tui::views::transcript::Transcript,
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
        let place = crate::tui::views::status_facts::display_path(&project, std::env::var("HOME").ok().as_deref());
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
            transcript: crate::tui::views::transcript::Transcript::default(),
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

    /// The dim text after the cursor that Right would put in: the suggested next prompt in an
    /// empty box, else the rest of a sent message (or an `@file`) that starts with what is
    /// typed. Only at the end of a one-line text, and never over a menu, card or history.
    pub fn ghost(&self) -> Option<String> {
        if self.menu.is_some() || self.approval.is_some() || self.confirm.is_some() || self.history.browsing() {
            return None;
        }
        let text = self.editor.text();
        if text.is_empty() {
            return self.suggestion.clone().filter(|_| self.working.is_none());
        }
        if self.editor.cursor() != text.len() || text.contains('\n') {
            return None;
        }
        crate::tui::input::ghost::ghost_suffix(text, self.history.newest_first(), &self.tracked)
    }

    /// Right at the end of the text: take the ghost text. False when there is none.
    fn accept_ghost(&mut self) -> bool {
        let Some(rest) = self.ghost() else { return false };
        self.editor.insert_str(&rest);
        self.edited();
        true
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

    fn refresh_menu(&mut self) {
        if self.menu_closed_for.as_deref() == Some(self.editor.text()) {
            self.menu = None;
            return;
        }
        self.menu_closed_for = None;
        let at_end = self.editor.cursor() == self.editor.text().len();
        self.menu = if at_end { menu::build(self.editor.before_cursor(), &self.ctx, &self.project, &self.tracked) } else { None };
    }

    /// The text changed by a key: browsing history stops and the menu follows the text.
    fn edited(&mut self) {
        self.history.reset();
        self.refresh_menu();
    }

    /// A message from the session.
    pub fn on_front(&mut self, msg: FrontMsg) -> Vec<Action> {
        match msg {
            FrontMsg::Print(text) => return vec![Action::Print(text)],
            FrontMsg::TurnStarted => {
                self.live_answer = None;
                self.working = Some(Working { label: "thinking".into(), activity: None, started: Instant::now() });
            }
            FrontMsg::TurnEnded => {
                self.live_answer = None;
                self.working = None;
                self.approval = None;
            }
            FrontMsg::Label(label) => {
                if let Some(w) = self.working.as_mut() {
                    w.label = label;
                    w.activity = None;
                }
            }
            FrontMsg::Activity(text) => {
                if let Some(w) = self.working.as_mut() {
                    w.activity = Some(text);
                }
            }
            FrontMsg::Approval(ask) => self.approval = Some(Approval::new(*ask, Instant::now())),
            FrontMsg::ApprovalGone => self.approval = None,
            FrontMsg::Confirm { question, reply } => self.confirm = Some(Confirm { question, reply: Some(reply), selected: 1 }),
            FrontMsg::Suspend { ack } => {
                self.suspended = true;
                return vec![Action::Suspend(ack)];
            }
            FrontMsg::Resume => {
                self.suspended = false;
                return vec![Action::Resume];
            }
            FrontMsg::ClearScreen => {
                self.live_answer = None;
                return vec![Action::ClearScreen];
            }
            FrontMsg::Bell => return vec![Action::Bell],
            FrontMsg::AnswerDelta(text) => match self.live_answer.as_mut() {
                Some(live) => live.push_str(&text),
                None => self.live_answer = Some(text),
            },
            FrontMsg::AnswerReset => self.live_answer = None,
            FrontMsg::Steered(text) => {
                if let Some(at) = self.queue.iter().position(|q| *q == text) {
                    self.queue.remove(at);
                }
            }
        }
        Vec::new()
    }

    /// Pasted text goes into the box as one piece (line breaks kept).
    pub fn on_paste(&mut self, text: &str) {
        if self.approval.is_none() && self.confirm.is_none() {
            self.editor.insert_str(text);
            self.edited();
        }
    }

    /// One key press.
    pub fn on_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        if k.kind == KeyEventKind::Release {
            return Vec::new();
        }
        if self.approval.as_ref().is_some_and(Approval::withdrawn) {
            self.approval = None;
        }
        if self.confirm.is_some() {
            return self.confirm_key(k);
        }
        if self.approval.is_some() {
            return self.approval_key(k, now);
        }
        if self.menu.is_some() {
            if let Some(actions) = self.menu_key(&k) {
                return actions;
            }
        }
        self.edit_key(k, now)
    }

    fn confirm_key(&mut self, k: KeyEvent) -> Vec<Action> {
        let Some(c) = self.confirm.as_mut() else { return Vec::new() };
        let answer = match k.code {
            KeyCode::Char('y' | 'Y') => Some(true),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(false),
            KeyCode::Char('c') if ctrl(&k) => Some(false),
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                c.selected = 1 - c.selected;
                None
            }
            KeyCode::Enter => Some(c.selected == 0),
            _ => None,
        };
        if let Some(yes) = answer {
            c.answer(yes);
            self.confirm = None;
        }
        Vec::new()
    }

    fn approval_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        let Some(a) = self.approval.as_mut() else { return Vec::new() };
        let stop = matches!(k.code, KeyCode::Char('c')) && ctrl(&k);
        if !stop && now.saturating_duration_since(a.shown_at) < CARD_GRACE {
            return Vec::new();
        }
        let choices = a.choices();
        let pick = |key: AnswerKey| choices.iter().any(|(c, _)| *c == key).then_some(key);
        let answer = match k.code {
            KeyCode::Char('c') if ctrl(&k) => Some(AnswerKey::Interrupt),
            KeyCode::Char('y' | 'Y') => Some(AnswerKey::Yes),
            KeyCode::Char('a' | 'A') => pick(AnswerKey::Always),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(AnswerKey::No),
            KeyCode::Char(c @ '1'..='9') => c.to_digit(10).and_then(|d| choices.get(d as usize - 1)).map(|(key, _)| *key),
            KeyCode::Up => {
                a.selected = (a.selected + choices.len() - 1) % choices.len();
                None
            }
            KeyCode::Down | KeyCode::Tab => {
                a.selected = (a.selected + 1) % choices.len();
                None
            }
            KeyCode::PageUp => {
                a.scroll = a.scroll.saturating_sub(5);
                None
            }
            KeyCode::PageDown => {
                a.scroll = (a.scroll + 5).min(a.lines.len().saturating_sub(1));
                None
            }
            KeyCode::Enter => choices.get(a.selected).map(|(key, _)| *key),
            _ => None,
        };
        if let Some(key) = answer {
            a.answer(key);
            self.approval = None;
        }
        Vec::new()
    }

    /// Keys the open menu takes; `None` passes the key on to the editor.
    fn menu_key(&mut self, k: &KeyEvent) -> Option<Vec<Action>> {
        let m = self.menu.as_mut()?;
        match k.code {
            KeyCode::Up => m.up(),
            KeyCode::Down => m.down(),
            KeyCode::Char('p') if ctrl(k) => m.up(),
            KeyCode::Char('n') if ctrl(k) => m.down(),
            KeyCode::Esc => {
                self.menu = None;
                self.menu_closed_for = Some(self.editor.text().to_string());
            }
            KeyCode::Tab => {
                self.accept(false);
            }
            KeyCode::Enter if k.modifiers.is_empty() => {
                if self.accept(true) {
                    return Some(self.submit());
                }
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// Put the selected item in the text. True when Enter should run the line now.
    fn accept(&mut self, enter: bool) -> bool {
        let Some(m) = self.menu.take() else { return false };
        let Some(item) = m.current().cloned() else { return false };
        let typed = self.editor.before_cursor()[m.from..].to_string();
        let already = typed == item.replacement;
        // Enter runs what was typed when it is complete already (`/permissions`, `/resume`),
        // and never swaps in a different command found by scattered letters (`/provders` is
        // sent as typed, to be told what was meant) unless the person picked it.
        if enter && (already || (!m.navigated && !item.replacement.to_lowercase().starts_with(&typed.to_lowercase()))) {
            return true;
        }
        let suffix = match item.then {
            Then::Arguments => " ",
            Then::Run | Then::Stay => "",
        };
        self.editor.replace_before_cursor(m.from, &format!("{}{suffix}", item.replacement));
        self.edited();
        match item.then {
            Then::Run => enter,
            // A file path chosen with Enter is put in; Enter again on the same text sends.
            Then::Stay => enter && already,
            Then::Arguments => false,
        }
    }

    /// Enter: send the text, or queue it while a turn runs.
    fn submit(&mut self) -> Vec<Action> {
        let text = self.editor.take();
        self.menu = None;
        self.menu_closed_for = None;
        if text.trim().is_empty() {
            return Vec::new();
        }
        // Anything sent answers the last answer: its offer no longer stands.
        self.suggestion = None;
        self.history.remember(&text);
        if self.working.is_some() {
            self.queue.push_back(text.clone());
            // The turn may take it in right away; it stays queued until the model has it.
            return vec![Action::Steer(text)];
        }
        vec![Action::Submit(text)]
    }

    fn newline(&mut self) {
        self.editor.insert_char('\n');
        self.edited();
    }

    fn edit_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        let busy = self.working.is_some();
        if let Some(action) = self.keymap.action(&k) {
            return match action {
                UserAction::Clear => vec![Action::ClearScreen],
                UserAction::CycleMode => {
                    self.mode = self.mode.next();
                    vec![Action::Mode(self.mode)]
                }
                UserAction::Quit => vec![Action::Quit],
            };
        }
        if let Some(vim) = self.vim.as_mut() {
            match vim.on_key(&mut self.editor, &k, busy) {
                Outcome::Pass => {}
                Outcome::Moved => return Vec::new(),
                Outcome::Edited => {
                    self.edited();
                    return Vec::new();
                }
            }
        }
        match k.code {
            // `?` on an empty box opens the key cheat sheet.
            KeyCode::Char('?') if self.editor.is_empty() && !busy && !ctrl(&k) => {
                self.editor.set("/keys");
                return self.submit();
            }
            KeyCode::Enter if k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => self.newline(),
            KeyCode::Char('j') if ctrl(&k) => self.newline(),
            KeyCode::Enter => {
                // A line ending in `\` continues on the next one, as in a shell.
                if self.editor.before_cursor().ends_with('\\') && self.editor.cursor() == self.editor.text().len() {
                    self.editor.backspace();
                    self.newline();
                } else {
                    return self.submit();
                }
            }
            KeyCode::Char('c') if ctrl(&k) => return self.ctrl_c(now),
            KeyCode::Esc if busy => {
                // Stop now; what was typed goes first, right after the stop.
                let text = self.editor.take();
                if !text.trim().is_empty() {
                    self.history.remember(&text);
                    self.queue.push_front(text);
                }
                self.edited();
                return vec![Action::Stop];
            }
            KeyCode::Esc => {}
            KeyCode::Char('d') if ctrl(&k) => {
                if self.editor.is_empty() && !busy {
                    return vec![Action::Quit];
                }
                self.editor.delete();
                self.edited();
            }
            KeyCode::BackTab => {
                self.mode = self.mode.next();
                return vec![Action::Mode(self.mode)];
            }
            KeyCode::Char('l') if ctrl(&k) => return vec![Action::ClearScreen],
            KeyCode::Char('r') if ctrl(&k) => {
                let needle = self.editor.text().to_string();
                if let Some(found) = self.history.search(&needle, &needle) {
                    self.editor.set(&found);
                    self.menu = None;
                }
            }
            KeyCode::Up if !self.editor.up() => {
                // A message sent during the turn that is still queued comes back before
                // history does: ↑ takes the newest one out of the queue. Only into an
                // empty box, so a draft is never thrown away for it.
                if self.editor.is_empty() {
                    if let Some(queued) = self.queue.pop_back() {
                        self.editor.set(&queued);
                        self.menu = None;
                        return Vec::new();
                    }
                }
                if let Some(older) = self.history.older(self.editor.text()) {
                    self.editor.set(&older);
                    self.menu = None;
                }
            }
            KeyCode::Down if !self.editor.down() => {
                if let Some(newer) = self.history.newer() {
                    self.editor.set(&newer);
                    self.menu = None;
                }
            }
            KeyCode::Left if ctrl(&k) || alt(&k) => self.editor.word_left(),
            KeyCode::Right if ctrl(&k) || alt(&k) => self.editor.word_right(),
            KeyCode::Char('b') if alt(&k) => self.editor.word_left(),
            KeyCode::Char('f') if alt(&k) => self.editor.word_right(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right if !self.accept_ghost() => self.editor.right(),
            KeyCode::Char('b') if ctrl(&k) => self.editor.left(),
            KeyCode::Char('f') if ctrl(&k) => self.editor.right(),
            KeyCode::PageUp => self.scroll_chat(self.pane_rows.saturating_sub(2).max(1) as isize),
            KeyCode::PageDown => self.scroll_chat(-(self.pane_rows.saturating_sub(2).max(1) as isize)),
            KeyCode::End if ctrl(&k) => self.transcript.to_end(),
            KeyCode::Home if ctrl(&k) => self.transcript.to_start(self.pane_rows),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            KeyCode::Char('a') if ctrl(&k) => self.editor.home(),
            KeyCode::Char('e') if ctrl(&k) => self.editor.end(),
            KeyCode::Backspace if alt(&k) || ctrl(&k) => {
                self.editor.delete_word_back();
                self.edited();
            }
            KeyCode::Char('w') if ctrl(&k) => {
                self.editor.delete_word_back();
                self.edited();
            }
            KeyCode::Char('k') if ctrl(&k) => {
                self.editor.kill_to_end();
                self.edited();
            }
            KeyCode::Char('u') if ctrl(&k) => {
                self.editor.kill_to_start();
                self.edited();
            }
            KeyCode::Char('h') if ctrl(&k) => {
                self.editor.backspace();
                self.edited();
            }
            KeyCode::Backspace => {
                self.editor.backspace();
                self.edited();
            }
            KeyCode::Delete => {
                self.editor.delete();
                self.edited();
            }
            KeyCode::Tab => self.refresh_menu(),
            KeyCode::Char(c) if !ctrl(&k) => {
                self.editor.insert_char(c);
                self.edited();
            }
            _ => {}
        }
        // Any key but Ctrl-C ends the "press again to exit" window.
        self.last_ctrl_c = None;
        Vec::new()
    }

    /// Ctrl-C: stop a running turn (and drop the queue); at the prompt, clear the text, and on
    /// an empty box twice within the window, exit.
    fn ctrl_c(&mut self, now: Instant) -> Vec<Action> {
        if self.working.is_some() {
            let dropped = self.queue.len();
            self.queue.clear();
            let mut out = vec![Action::Stop];
            if dropped > 0 {
                let noun = if dropped == 1 { "message" } else { "messages" };
                out.push(Action::Print(format!("  dropped {dropped} queued {noun} (Ctrl-C); Esc stops and keeps them")));
            }
            return out;
        }
        if !self.editor.is_empty() {
            self.editor.take();
            self.edited();
            self.last_ctrl_c = None;
            return Vec::new();
        }
        if self.last_ctrl_c.is_some_and(|t| now.duration_since(t) < INTERRUPT_WINDOW) {
            return vec![Action::Quit];
        }
        self.last_ctrl_c = Some(now);
        self.notice = Some(("Press Ctrl-C again to exit".into(), now));
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn app() -> App {
        let mut a = App::new(Path::new("/nonexistent").into(), History::from_entries(&["older message"]), Mode::Edit, "auto".into(), Vec::new());
        a.ctx = CompleteCtx { ready_providers: vec!["claude".into()], ..CompleteCtx::default() };
        a
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn with(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn typed(a: &mut App, text: &str) -> Vec<Action> {
        let mut out = Vec::new();
        for c in text.chars() {
            out.extend(a.on_key(key(KeyCode::Char(c)), Instant::now()));
        }
        out
    }

    #[test]
    fn right_takes_the_suggested_next_prompt_and_sending_clears_it() {
        let mut a = app();
        a.suggestion = Some("commit this and tag v1.1.0".into());
        assert_eq!(a.ghost().as_deref(), Some("commit this and tag v1.1.0"));
        a.on_key(key(KeyCode::Right), Instant::now());
        assert_eq!(a.editor.text(), "commit this and tag v1.1.0");
        assert_eq!(a.ghost(), None, "nothing left to add");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("commit this and tag v1.1.0"));
        assert_eq!((a.suggestion.as_deref(), a.ghost()), (None, None));
    }

    #[test]
    fn the_suggestion_waits_for_an_empty_idle_box_and_typing_something_else_hides_it() {
        let mut a = app();
        a.suggestion = Some("push the branch".into());
        a.working = Some(Working { label: "thinking".into(), activity: None, started: Instant::now() });
        assert_eq!(a.ghost(), None, "not while a turn runs");
        a.working = None;
        typed(&mut a, "zz");
        assert_eq!(a.ghost(), None, "typed text that matches nothing has no ghost");
        a.on_key(key(KeyCode::Backspace), Instant::now());
        a.on_key(key(KeyCode::Backspace), Instant::now());
        assert_eq!(a.ghost().as_deref(), Some("push the branch"), "an empty box shows it again");
        // A typed message replaces the offer once sent.
        typed(&mut a, "something else");
        a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(a.suggestion, None);
    }

    #[test]
    fn typed_text_ghosts_the_newest_matching_message_and_right_takes_it_only_at_the_end() {
        let mut a = app();
        typed(&mut a, "older");
        assert_eq!(a.ghost().as_deref(), Some(" message"));
        a.on_key(key(KeyCode::Left), Instant::now());
        assert_eq!(a.ghost(), None, "not with the cursor inside the text");
        a.on_key(key(KeyCode::Right), Instant::now());
        assert_eq!((a.editor.text(), a.editor.cursor()), ("older", 5), "Right moves the cursor when there is no ghost");
        a.on_key(key(KeyCode::Right), Instant::now());
        assert_eq!(a.editor.text(), "older message");
    }

    fn submitted(actions: &[Action]) -> Option<&str> {
        actions.iter().find_map(|a| match a {
            Action::Submit(t) => Some(t.as_str()),
            _ => None,
        })
    }

    #[test]
    fn enter_sends_and_shift_or_alt_enter_or_ctrl_j_or_backslash_adds_a_line() {
        let mut a = app();
        typed(&mut a, "one");
        a.on_key(with(KeyCode::Enter, KeyModifiers::SHIFT), Instant::now());
        typed(&mut a, "two");
        a.on_key(with(KeyCode::Enter, KeyModifiers::ALT), Instant::now());
        a.on_key(with(KeyCode::Char('j'), KeyModifiers::CONTROL), Instant::now());
        typed(&mut a, "three\\");
        a.on_key(key(KeyCode::Enter), Instant::now());
        typed(&mut a, "four");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("one\ntwo\n\nthree\nfour"));
        assert!(a.editor.is_empty());
    }

    #[test]
    fn a_slash_opens_the_menu_and_enter_runs_a_complete_command() {
        let mut a = app();
        typed(&mut a, "/");
        assert_eq!(a.menu.as_ref().map(|m| m.items[0].replacement.as_str()), Some("/model"));
        typed(&mut a, "hel");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("/help"));
    }

    #[test]
    fn a_command_with_arguments_takes_a_space_and_shows_its_choices() {
        let mut a = app();
        typed(&mut a, "/mod");
        a.on_key(key(KeyCode::Down), Instant::now());
        let out = a.on_key(key(KeyCode::Tab), Instant::now());
        assert!(out.is_empty());
        assert_eq!(a.editor.text(), "/mode ");
        let choices: Vec<_> = a.menu.as_ref().map(|m| m.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default();
        assert_eq!(choices, vec!["ask", "edit", "plan", "auto"]);
        a.on_key(key(KeyCode::Down), Instant::now());
        a.on_key(key(KeyCode::Down), Instant::now());
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("/mode plan"));
    }

    #[test]
    fn esc_closes_the_menu_until_the_text_changes() {
        let mut a = app();
        typed(&mut a, "/c");
        assert!(a.menu.is_some());
        a.on_key(key(KeyCode::Esc), Instant::now());
        assert!(a.menu.is_none());
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("/c"), "with the menu closed Enter sends the text as typed");
        typed(&mut a, "/c");
        assert!(a.menu.is_some(), "new text, new menu");
    }

    #[test]
    fn a_message_the_model_took_into_the_turn_leaves_the_queue() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        a.queue.extend(["first".to_string(), "second".to_string()]);
        a.on_front(FrontMsg::Steered("first".into()));
        assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["second".to_string()]);
        // A message nobody queued changes nothing.
        a.on_front(FrontMsg::Steered("other".into()));
        assert_eq!(a.queue.len(), 1);
    }

    #[test]
    fn typing_during_a_turn_queues_and_esc_stops_sending_the_text_first() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        typed(&mut a, "later");
        // Queued, and offered to the running turn.
        assert!(matches!(a.on_key(key(KeyCode::Enter), Instant::now()).as_slice(), [Action::Steer(t)] if t == "later"));
        typed(&mut a, "now");
        let out = a.on_key(key(KeyCode::Esc), Instant::now());
        assert!(matches!(out.as_slice(), [Action::Stop]));
        assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["now".to_string(), "later".to_string()]);
    }

    #[test]
    fn ctrl_c_stops_a_turn_and_drops_the_queue_and_twice_on_an_empty_box_exits() {
        let mut a = app();
        let ctrl_c = with(KeyCode::Char('c'), KeyModifiers::CONTROL);
        a.on_front(FrontMsg::TurnStarted);
        a.queue.push_back("queued".into());
        let out = a.on_key(ctrl_c, Instant::now());
        assert!(matches!(out.as_slice(), [Action::Stop, Action::Print(_)]));
        assert!(a.queue.is_empty());
        a.on_front(FrontMsg::TurnEnded);

        typed(&mut a, "draft");
        assert!(a.on_key(ctrl_c, Instant::now()).is_empty());
        assert!(a.editor.is_empty(), "the first Ctrl-C clears the text");
        let t = Instant::now();
        assert!(a.on_key(ctrl_c, t).is_empty());
        assert_eq!(a.notice.as_ref().map(|n| n.0.as_str()), Some("Press Ctrl-C again to exit"));
        assert!(matches!(a.on_key(ctrl_c, t + Duration::from_millis(500)).as_slice(), [Action::Quit]));
    }

    #[test]
    fn up_browses_history_and_down_comes_back() {
        let mut a = app();
        typed(&mut a, "draft");
        a.on_key(key(KeyCode::Up), Instant::now());
        assert_eq!(a.editor.text(), "older message");
        a.on_key(key(KeyCode::Down), Instant::now());
        assert_eq!(a.editor.text(), "draft");
    }

    #[test]
    fn up_takes_the_last_queued_message_back_into_the_box() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        typed(&mut a, "first");
        a.on_key(key(KeyCode::Enter), Instant::now());
        typed(&mut a, "second");
        a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["first".to_string(), "second".to_string()]);

        a.on_key(key(KeyCode::Up), Instant::now());
        assert_eq!(a.editor.text(), "second", "↑ pops the newest queued message");
        assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["first".to_string()]);

        // The box holds that one now; clearing it lets ↑ take the other.
        a.on_key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), Instant::now());
        a.on_key(key(KeyCode::Up), Instant::now());
        assert_eq!(a.editor.text(), "first");
        assert!(a.queue.is_empty());

        // With nothing queued, ↑ is history again (the messages sent above are in it).
        a.on_key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), Instant::now());
        a.on_key(key(KeyCode::Up), Instant::now());
        assert_eq!(a.editor.text(), "second");
    }

    #[test]
    fn a_draft_in_the_box_is_not_thrown_away_for_a_queued_message() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        a.queue.push_back("queued".into());
        typed(&mut a, "draft");
        a.on_key(key(KeyCode::Up), Instant::now());
        assert_eq!(a.editor.text(), "older message", "↑ is history while the box has a draft");
        assert_eq!(a.queue.len(), 1);
    }

    #[test]
    fn shift_tab_cycles_the_mode() {
        let mut a = app();
        let out = a.on_key(key(KeyCode::BackTab), Instant::now());
        assert!(matches!(out.as_slice(), [Action::Mode(Mode::Plan)]), "edit → plan");
    }

    fn ask(always: bool) -> (ApprovalAsk, oneshot::Receiver<AnswerKey>) {
        let (reply, rx) = oneshot::channel();
        let ask = ApprovalAsk {
            tool: "Bash".into(),
            notes: Vec::new(),
            lines: vec!["cargo test".into()],
            always: always.then(|| "Bash(cargo test:*)".to_string()),
            note: None,
            reply,
        };
        (ask, rx)
    }

    #[test]
    fn the_approval_card_answers_by_number_letter_arrows_or_esc() {
        for (keys, expected) in [
            (vec![key(KeyCode::Char('1'))], AnswerKey::Yes),
            (vec![key(KeyCode::Char('2'))], AnswerKey::Always),
            (vec![key(KeyCode::Char('3'))], AnswerKey::No),
            (vec![key(KeyCode::Char('a'))], AnswerKey::Always),
            (vec![key(KeyCode::Esc)], AnswerKey::No),
            (vec![key(KeyCode::Down), key(KeyCode::Enter)], AnswerKey::Always),
            (vec![key(KeyCode::Up), key(KeyCode::Enter)], AnswerKey::No),
            (vec![with(KeyCode::Char('c'), KeyModifiers::CONTROL)], AnswerKey::Interrupt),
        ] {
            let mut a = app();
            let (q, mut rx) = ask(true);
            a.on_front(FrontMsg::Approval(Box::new(q)));
            let later = Instant::now() + CARD_GRACE;
            for k in keys {
                a.on_key(k, later);
            }
            assert_eq!(rx.try_recv().ok(), Some(expected));
            assert!(a.approval.is_none());
        }
    }

    #[test]
    fn without_always_on_offer_a_and_2_do_not_save_a_rule() {
        let mut a = app();
        let (q, mut rx) = ask(false);
        a.on_front(FrontMsg::Approval(Box::new(q)));
        let later = Instant::now() + CARD_GRACE;
        a.on_key(key(KeyCode::Char('a')), later);
        assert!(rx.try_recv().is_err(), "a is not an answer");
        a.on_key(key(KeyCode::Char('2')), later);
        assert_eq!(rx.try_recv().ok(), Some(AnswerKey::No), "2 is the second choice: No");
    }

    #[test]
    fn keys_typed_ahead_never_answer_a_card_that_just_appeared_but_ctrl_c_stops() {
        let mut a = app();
        let (q, mut rx) = ask(true);
        a.on_front(FrontMsg::Approval(Box::new(q)));
        a.on_key(key(KeyCode::Enter), Instant::now());
        a.on_key(key(KeyCode::Char('y')), Instant::now());
        assert!(rx.try_recv().is_err() && a.approval.is_some(), "Enter or y right away is typing ahead");
        a.on_key(with(KeyCode::Char('c'), KeyModifiers::CONTROL), Instant::now());
        assert_eq!(rx.try_recv().ok(), Some(AnswerKey::Interrupt));
    }

    #[test]
    fn enter_never_swaps_in_a_command_found_by_scattered_letters() {
        let mut a = app();
        typed(&mut a, "/provders");
        assert!(a.menu.is_some(), "the menu offers /providers");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("/provders"), "sent as typed, to be told what was meant");
        typed(&mut a, "/provders");
        a.on_key(key(KeyCode::Down), Instant::now());
        a.on_key(key(KeyCode::Up), Instant::now());
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), None, "picked on purpose: put in, takes arguments");
        assert_eq!(a.editor.text(), "/providers ");
    }

    #[test]
    fn enter_on_a_complete_command_with_optional_arguments_runs_it() {
        let mut a = app();
        typed(&mut a, "/permissions");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("/permissions"));
    }

    #[test]
    fn a_withdrawn_question_is_dropped_and_the_key_goes_to_the_box() {
        let mut a = app();
        let (q, rx) = ask(true);
        a.on_front(FrontMsg::Approval(Box::new(q)));
        drop(rx);
        typed(&mut a, "y");
        assert!(a.approval.is_none());
        assert_eq!(a.editor.text(), "y");
    }

    #[test]
    fn a_confirm_defaults_to_no_and_takes_y() {
        let mut a = app();
        let (reply, rx) = std_mpsc::channel();
        a.on_front(FrontMsg::Confirm { question: "Revert 2 files?".into(), reply });
        a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(rx.try_recv().ok(), Some(false));
        let (reply, rx) = std_mpsc::channel();
        a.on_front(FrontMsg::Confirm { question: "Revert?".into(), reply });
        a.on_key(key(KeyCode::Char('y')), Instant::now());
        assert_eq!(rx.try_recv().ok(), Some(true));
    }

    #[test]
    fn a_question_mark_on_an_empty_box_opens_the_cheat_sheet_but_is_text_elsewhere() {
        let mut a = app();
        assert_eq!(submitted(&a.on_key(key(KeyCode::Char('?')), Instant::now())), Some("/keys"));
        let mut b = app();
        typed(&mut b, "why?");
        assert_eq!(b.editor.text(), "why?");
    }

    #[test]
    fn vim_mode_moves_and_edits_in_normal_mode_and_sends_on_enter() {
        let mut a = app();
        a.vim = Some(Vim::default());
        typed(&mut a, "hello");
        a.on_key(key(KeyCode::Esc), Instant::now());
        typed(&mut a, "0x");
        assert_eq!(a.editor.text(), "ello");
        typed(&mut a, "A!");
        assert_eq!(a.editor.text(), "ello!");
        a.on_key(key(KeyCode::Esc), Instant::now());
        assert_eq!(submitted(&a.on_key(key(KeyCode::Enter), Instant::now())), Some("ello!"));
    }

    #[test]
    fn a_user_binding_beats_the_default_and_runs_its_action() {
        let mut a = app();
        a.keymap = Keymap::parse("ctrl-o = clear\nf2 = quit\n").0;
        let out = a.on_key(with(KeyCode::Char('o'), KeyModifiers::CONTROL), Instant::now());
        assert!(matches!(out.as_slice(), [Action::ClearScreen]));
        assert!(matches!(a.on_key(key(KeyCode::F(2)), Instant::now()).as_slice(), [Action::Quit]));
    }

    #[test]
    fn paste_keeps_its_lines_in_one_message() {
        let mut a = app();
        a.on_paste("line one\nline two");
        let out = a.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(submitted(&out), Some("line one\nline two"));
    }
}
