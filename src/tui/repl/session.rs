//! The session: what each typed command does, and how one turn runs. The binary reads
//! lines (from the line editor or a pipe) and hands them to [`Repl::handle`]; everything
//! that follows is here, over an [`Engine`] and a [`Console`], so it is tested with fake
//! backends instead of a terminal.
//!
//! The command dispatch is in `handle.rs`, one turn in `turn.rs`, the Ctrl-C race in
//! `turn_race.rs`; the commands that read or change the setup are in `commands.rs`,
//! `saved_session.rs` and `undo.rs`.

use crate::tui::console::terminal_ui::Console;
use crate::tui::repl::library_cmds;
use crate::shared::infrastructure::jsonl_log as log;
use crate::tui::render as uifmt;
use crate::tui::render::Style;
use crate::turn::application::engine::Engine;
use crate::turn::domain::session_log::SessionLog;

mod commands;
mod handle;
mod saved_session;
mod turn;
mod undo;
pub mod pager;
pub mod turn_race;
#[cfg(test)]
pub(crate) mod tests;

use turn_race::Interrupt;

/// Answers a yes/no question through the terminal.
pub trait Confirm: Send {
    fn confirm(&mut self, question: &str) -> bool;
}

/// What the loop does after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
    /// Exit at once (a second Ctrl-C during a stopping turn).
    Aborted,
}

/// The terminal-facing parts one command may need.
pub struct Io<'a> {
    pub interrupt: &'a mut dyn Interrupt,
    /// Present only in an interactive session; without it `/undo` refuses.
    pub confirm: Option<&'a mut dyn Confirm>,
}

pub struct Repl {
    pub engine: Engine,
    pub console: Console,
    pub(crate) home: Option<String>,
    /// Whether stdout is a terminal, which decides if long output may go through a pager.
    pub(crate) paging: bool,
    pub(crate) last_summary: Option<uifmt::SummaryFacts>,
    /// The turns of this session, for `/cost`, `/export` and `/copy`.
    pub(crate) log: SessionLog,
    /// Notes that go with the next message: `!command` output, a `/compact` summary.
    pub(crate) carry: Vec<String>,
    /// Where scratch files go (`/edit`); the temp folder when `None`.
    pub(crate) state_dir: Option<std::path::PathBuf>,
    /// The last `/sessions` listing, which its numbers refer to.
    pub(crate) library: library_cmds::LibraryState,
}

impl Repl {
    pub fn new(engine: Engine, console: Console, home: Option<String>, paging: bool) -> Self {
        Self { engine, console, home, paging, last_summary: None, log: SessionLog::default(), carry: Vec::new(), state_dir: None, library: Default::default() }
    }

    /// Keep the mode the person just chose for the next session. Only their choices come
    /// here: the read-only mode `/review` and `/compact` borrow for one turn is not kept.
    pub fn remember_mode(&self) {
        if let Some(dir) = &self.state_dir {
            self.said_if_failed(crate::approval::application::mode_store::save(dir, self.engine.mode()), "the mode");
        }
    }

    /// Keep an on/off setting the person just chose (`/orchestrate`, `/provider`).
    fn remember_flag(&self, flag: crate::tui::app::preferences::Flag, on: bool, what: &str) {
        if let Some(dir) = &self.state_dir {
            self.said_if_failed(crate::tui::app::preferences::save_flag(dir, flag, on), what);
        }
    }

    fn said_if_failed(&self, saved: std::io::Result<()>, what: &str) {
        if let Err(e) = saved {
            self.console.err(&format!("  could not keep {what} for the next session: {e}"));
        }
    }

    /// Keep scratch files (the message `/edit` composes) in the state folder.
    pub fn with_state_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.state_dir = dir;
        self
    }

    /// Where the worker registry and scratch files live.
    pub fn state_dir(&self) -> Option<&std::path::Path> {
        self.state_dir.as_deref()
    }

    /// The session's turns so far.
    pub fn log(&self) -> &SessionLog {
        &self.log
    }

    pub(crate) fn now(&self) -> u64 {
        log::now_ms()
    }

    /// The worker records on disk (none without a state directory).
    fn stored_workers(&self) -> Vec<crate::orchestration::infrastructure::worker_file::StoredWorker> {
        log::state_dir().map(|d| crate::orchestration::infrastructure::worker_file::stored_workers(&d)).unwrap_or_default()
    }

    pub(crate) fn out_style(&self) -> Style {
        self.console.out_style()
    }

    /// Print command output, followed by a blank line.
    pub(crate) fn page_out(&self, text: &str) {
        self.console.out(text);
        self.console.out("");
    }

}
