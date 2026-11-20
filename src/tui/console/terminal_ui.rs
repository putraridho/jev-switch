//! The terminal output of one `jev-code` run: a [`Ui`] that prints a turn as it happens
//! (route, tool calls, notes, the answer), keeps a live status line under it while a model
//! works, and serialises every byte through ONE mutex so lines never interleave with the
//! status line (erase it, print, draw it again).
//!
//! All wording and styling comes from [`crate::tui::render`]; this module only decides what goes
//! where. Answers go to stdout, everything else (route, tools, notes, errors, the summary)
//! goes to stderr, so `jev-code -p ... > answer.txt` captures exactly the answer. The
//! writers are injected, so every behaviour is a unit test over byte buffers.

mod approval_prompt;
mod console_api;
pub mod key_reader;
pub mod lines;
mod output;
mod turn_events;
mod turn_observer;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crate::backend::{self, Decision, Event, PermissionRequest};
use crate::tui::console::front::{ApprovalAsk, Front, FrontMsg};
use crate::tui::input::line_editor as lineedit;
use crate::tui::input::line_editor::AnswerKey;
use crate::approval::domain::rule_policy as permissions;
use crate::approval::domain::rule_policy::{Offer, Review, Settled};
use crate::tui::tty::terminal_guard as terminal;
use crate::tui::tty::terminal_guard::{Screen, StdScreen};
use crate::turn::application::engine::{Answer, Failover, NoRouteInfo, Ui};
use crate::tui::render as uifmt;
use crate::tui::render::{FileChange, RouteFacts, Style, SummaryFacts, Theme};

use self::approval_prompt::*;
use self::key_reader::*;
use self::lines::*;

/// How often the status line is redrawn (the spinner frame changes every 100 ms).
pub const TICK: Duration = Duration::from_millis(100);

/// Home the cursor, clear the screen, and clear its scrollback: the whole display belongs to
/// this session. Shared by the startup clear, `/clear`, and Ctrl-L.
pub const CLEAR_SCREEN: &str = "\x1b[H\x1b[2J\x1b[3J";

/// What the console needs to know about its surroundings.
#[derive(Debug, Clone)]
pub struct ConsoleConfig {
    /// Styling for what goes to stdout (the answer, command output).
    pub out_style: Style,
    /// Styling for what goes to stderr (progress).
    pub err_style: Style,
    /// The width to lay lines out for when the terminal cannot be asked (tests, pipes). A
    /// console on a real terminal asks it again on every draw, so a resize is followed.
    pub width: usize,
    /// Draw the live status line (stderr is a terminal that takes escape sequences).
    pub live: bool,
    /// The project folder, to show paths relative to it.
    pub project: PathBuf,
    /// Print assistant text to stdout as it arrives. Off for `-p`, where the caller prints
    /// the final answer itself.
    pub stream_answers: bool,
}

struct Pending {
    provider: String,
    model: String,
    kind: crate::backend::ErrorKind,
    discarded: bool,
}

/// Shows a long approval input in full, on the terminal; false when it could not. Runs on a
/// blocking thread, so it gets the text by value.
pub type Pager = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Output that arrived while the pager owned the screen, printed when it closes.
enum HeldLine {
    Out(String),
    Err(String),
}

struct Live {
    label: String,
    activity: Option<String>,
    started: Instant,
    /// Rows the live area takes on screen right now: 0 when not drawn, 1 for the status line,
    /// 2 with the field under it. The cursor is left on the last of them.
    rows: usize,
}

struct Inner {
    out: Box<dyn Write + Send>,
    err: Box<dyn Write + Send>,
    cfg: ConsoleConfig,
    live: Option<Live>,
    /// An approval question is on screen: the status line stays away until it is answered
    /// or withdrawn. Holds how many rows the question takes while it is the last thing
    /// printed, so a withdrawal can erase it; 0 once anything else was printed after it.
    asking: Option<usize>,
    cursor_hidden: bool,
    /// The terminal itself, when there is one to ask; `None` in tests and pipes.
    screen: Option<Box<dyn Screen>>,
    /// Typed keys are not echoed while the status line is up; this is what the terminal had
    /// before the turn (`None`: not a terminal, or no turn running), put back at the end.
    echo_before: Option<bool>,
    /// Notices that arrived while the user was at the prompt, printed at the next input.
    deferred: Vec<String>,
    /// A failure whose "→ trying X" part is not known until the next route is chosen.
    pending: Option<Pending>,
    /// Set when no model could take the turn; the caller prints the full message, which
    /// needs facts the console does not have.
    no_route_task: Option<String>,
    /// The pager owns the screen: everything printed meanwhile waits here (in order) and the
    /// status line stays away; `None` when no pager is open.
    held: Option<Vec<HeldLine>>,
    /// The route of the latest turn, for `/why`.
    last_route: Option<RouteFacts>,
    /// What is being typed during a turn, shown in the field under the status line.
    draft: String,
    /// Keys are read during a turn, so the field under the status line is drawn.
    type_ahead: bool,
    /// Messages typed during a turn, to run when it is over (oldest first).
    queued: std::collections::VecDeque<String>,
    /// The interactive front end, when one owns the screen: everything goes to it as a
    /// message and nothing is drawn here.
    front: Option<Front>,
    /// With a front: the latest assistant text, not printed yet. What comes next decides how
    /// it looks: a tool call makes it narration (the model thinking aloud), the end of the
    /// turn makes it the answer, a failed attempt drops it.
    pending_text: Option<String>,
    /// With a front: the answer as it is being written, for the live preview only. The
    /// committed answer still comes from `pending_text` / the final `Text`.
    live_answer: String,
    /// With a front: the tools called whose results have not come back, oldest first, so a
    /// result is worded for its tool.
    tools_waiting: std::collections::VecDeque<String>,
    /// With a front: the tool calls of the run in progress, held back until it ends so the run
    /// prints as one summary row (see [`Inner::flush_run`]).
    run: Vec<HeldCall>,
    /// The run in progress counted in words, for that row: what was edited, read, run.
    group: uifmt::ToolGroup,
    /// With a front: nothing of this turn printed yet (the first step needs no blank line).
    fresh_turn: bool,
    /// Where messages typed during the turn go to be added to it ([`Ui::steering`]); a send
    /// that fails means the turn is over.
    steer: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Whether the model's whole reasoning is printed. `/thinking` sets it; the
    /// `JEV_SHOW_THINKING` environment variable seeds it.
    show_thinking: bool,
}

/// The terminal output. Cheap to clone; every clone writes through the same lock.
#[derive(Clone)]
pub struct Console {
    inner: Arc<Mutex<Inner>>,
    /// How to ask the person a question. `None` (no terminal to answer on) means every
    /// tool approval is denied at once.
    keys: Option<KeyReader>,
    /// True while the pager of an approval question is open (see [`Console::is_paging`]).
    paging: Arc<tokio::sync::watch::Sender<bool>>,
    /// One pager at a time: two questions that both need one take turns.
    pager_turn: Arc<tokio::sync::Mutex<()>>,
    /// Read keys during a turn ([`crate::tui::input::typeahead`]): only a person at a terminal.
    type_ahead: bool,
}

/// A tool call held back while the run it belongs to is still going. Only a failed call
/// prints rows of its own (under the run's row); the rest are counted.
struct HeldCall {
    tool: String,
    /// The call as the CLI described it: `Bash cargo test`.
    summary: String,
    /// What it prints under the run's row: set when it failed.
    rows: Vec<String>,
    resolved: bool,
    failed: bool,
}
