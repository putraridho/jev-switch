//! Owns what is known about the terminals and the console built from it. Must not read
//! arguments or start a session.

use std::io::IsTerminal;
use std::path::Path;
use crate::console::terminal_ui::{Console, ConsoleConfig};
use crate::console::front::Front;
use crate::tty::terminal_guard as terminal;
use crate::render::Style;

/// What is known about the terminals before anything is printed.
pub(super) struct Term {
    pub(super) stdin_tty: bool,
    pub(super) stdout_tty: bool,
    pub(super) out_style: Style,
    pub(super) err_style: Style,
    pub(super) width: usize,
}

impl Term {
    pub(super) fn detect(no_color: bool) -> Self {
        let (stdin_tty, stdout_tty, stderr_tty) =
            (std::io::stdin().is_terminal(), std::io::stdout().is_terminal(), std::io::stderr().is_terminal());
        let style = |tty: bool| if no_color { Style::off() } else { Style::detect(tty) };
        // A width only means something on a terminal; elsewhere lines are never cut.
        let width = if stdout_tty || stderr_tty { terminal::width() } else { 4096 };
        Self { stdin_tty, stdout_tty, out_style: style(stdout_tty), err_style: style(stderr_tty), width }
    }

    /// Whether this is a person at a keyboard: the line editor, the banner hints and the
    /// first-run hint are for them only.
    pub(super) fn interactive(&self) -> bool {
        self.stdin_tty && self.stdout_tty
    }

    /// The live status line needs a terminal on stderr that takes escape sequences.
    pub(super) fn live(&self) -> bool {
        std::io::stderr().is_terminal() && self.err_style.enabled
    }
}

/// `stream_answers` is false for `-p`, which also never asks: a one-shot run behaves the
/// same on a terminal and in a pipe. A session on a terminal hands the screen to the
/// interactive front end (`front`), which draws everything and asks about each tool call on
/// a card.
pub(super) fn console_for(term: &Term, project: &Path, stream_answers: bool, front: Option<Front>) -> Console {
    let console = Console::stdio(ConsoleConfig {
        out_style: term.out_style,
        err_style: term.err_style,
        width: term.width,
        live: term.live() && front.is_none(),
        project: project.to_path_buf(),
        stream_answers,
    });
    if let Some(front) = front {
        return console.with_front(front);
    }
    if term.interactive() && stream_answers {
        let console = console.with_key_reader(crate::console::terminal_ui::key_reader::terminal_keys());
        // What is typed during a turn shows on the status line: without one it would be blind.
        if term.live() {
            console.with_type_ahead()
        } else {
            console
        }
    } else {
        console
    }
}
