//! The line editor of the interactive session: history that survives restarts, Tab
//! completion and an inline hint from the command table in [`crate::tui::render`], a prompt that
//! shows the routing state, Alt-Enter for a new line, and the "Ctrl-C twice to exit" rule.
//!
//! The decisions (what completes, what the hint says, whether this Ctrl-C exits) are pure
//! and tested; the [`rustyline`] glue around them is thin.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Cmd, ColorMode, CompletionType, Config, Context, Editor, EventHandler, Helper, KeyCode, KeyEvent, Modifiers};

use crate::approval::domain::mode::SharedMode;

use crate::tui::render as uifmt;
use crate::tui::render::{CompleteCtx, Style};

use history::{prepare_history_file, HISTORY_MAX};
use keys::{BackslashNewline, CycleMode};

pub mod approval;
pub mod completion;
pub mod history;
pub mod interrupt;
mod keys;
#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------- helper

/// Completion, hint and prompt colouring for the editor.
pub struct JevHelper {
    ctx: CompleteCtx,
    style: Style,
    plain_prompt: String,
    styled_prompt: String,
    mode: SharedMode,
    /// The project, where `@path` completion looks; `None` completes no paths.
    project: Option<PathBuf>,
}

impl JevHelper {
    pub fn new(style: Style, mode: SharedMode) -> Self {
        Self { ctx: CompleteCtx::default(), style, plain_prompt: String::new(), styled_prompt: String::new(), mode, project: None }
    }

    /// Let Tab complete `@path` mentions from this folder.
    pub fn set_project(&mut self, dir: &Path) {
        self.project = Some(dir.to_path_buf());
    }

    /// What Tab may offer from now on (refreshed before every prompt).
    pub fn set_ctx(&mut self, ctx: CompleteCtx) {
        self.ctx = ctx;
    }

    /// The prompt for the next line: `plain` is what the editor measures, `styled` what it draws.
    pub fn set_prompt(&mut self, plain: &str, styled: &str) {
        self.plain_prompt = plain.to_string();
        self.styled_prompt = styled.to_string();
    }
}


/// The faint text on an empty line: the mode and how to change it.
pub fn mode_hint(mode: crate::approval::domain::mode::Mode) -> String {
    format!("[{mode} mode \u{b7} shift-tab to change]")
}

impl Hinter for JevHelper {
    type Hint = String;

    /// The faint rest of a command, only when the cursor is at the end of the line.
    fn hint(&self, line: &str, pos: usize, _: &Context<'_>) -> Option<String> {
        if pos < line.len() {
            return None;
        }
        // An empty line shows the mode, so Shift-Tab has something to change on screen.
        if line.is_empty() {
            return Some(mode_hint(self.mode.get()));
        }
        uifmt::hint(line)
    }
}

impl Highlighter for JevHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, default: bool) -> Cow<'b, str> {
        if default && self.style.enabled && prompt == self.plain_prompt && !self.styled_prompt.is_empty() {
            Cow::Borrowed(&self.styled_prompt)
        } else {
            Cow::Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if self.style.enabled {
            Cow::Owned(self.style.dim(hint))
        } else {
            Cow::Borrowed(hint)
        }
    }
}

impl Validator for JevHelper {}
impl Helper for JevHelper {}


/// The editor type of the session.
pub type LineEditor = Editor<JevHelper, DefaultHistory>;

/// Build the editor: history loaded from `history` (when given), Tab completion that lists
/// the choices, Alt-Enter for a new line, bracketed paste (multi-line text pastes as one
/// message) and no history entry unless [`remember`] adds one.
pub fn build_editor(style: Style, history: Option<&Path>, mode: SharedMode) -> rustyline::Result<LineEditor> {
    let config = Config::builder()
        .max_history_size(HISTORY_MAX)?
        .history_ignore_dups(true)?
        .auto_add_history(false)
        .completion_type(CompletionType::List)
        .color_mode(if style.enabled { ColorMode::Forced } else { ColorMode::Disabled })
        .bracketed_paste(true)
        .build();
    let mut editor: LineEditor = Editor::with_config(config)?;
    editor.set_helper(Some(JevHelper::new(style, mode.clone())));
    editor.bind_sequence(KeyEvent::alt('\r'), Cmd::Newline);
    editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::NONE), EventHandler::Conditional(Box::new(BackslashNewline)));
    editor.bind_sequence(KeyEvent(KeyCode::BackTab, Modifiers::NONE), EventHandler::Conditional(Box::new(CycleMode(mode))));
    if let Some(path) = history {
        prepare_history_file(path);
        let _ = editor.load_history(path);
    }
    Ok(editor)
}

// ------------------------------------------------------------------- one prompt

/// What one prompt produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    Line(String),
    /// Ctrl-C: the line was thrown away.
    Interrupted,
    /// Ctrl-D on an empty line.
    Eof,
    /// The terminal failed; the message is for the user.
    Failed(String),
}

/// Read one message. `plain` and `styled` are the prompt without and with colour; `ctx`
/// is what Tab may offer; `initial` is text already in the line (typed during a turn).
pub fn read(editor: &mut LineEditor, plain: &str, styled: &str, ctx: CompleteCtx, initial: &str) -> Input {
    if let Some(h) = editor.helper_mut() {
        h.set_ctx(ctx);
        h.set_prompt(plain, styled);
    }
    let read = if initial.is_empty() { editor.readline(plain) } else { editor.readline_with_initial(plain, (initial, "")) };
    match read {
        Ok(line) => Input::Line(line),
        Err(ReadlineError::Interrupted) => Input::Interrupted,
        Err(ReadlineError::Eof) => Input::Eof,
        Err(e) => Input::Failed(describe(&e)),
    }
}

/// A line-editor failure in plain words, never a `Debug` dump.
pub fn describe(e: &ReadlineError) -> String {
    match e {
        ReadlineError::Io(io) => format!("the terminal failed: {io}"),
        #[cfg(unix)]
        ReadlineError::Errno(errno) => format!("the terminal failed: {errno}"),
        other => format!("the terminal failed: {other}"),
    }
}

/// Ask a yes/no question; anything but `y`/`yes` (including Ctrl-C and Ctrl-D) is no.
pub fn confirm(editor: &mut LineEditor, question: &str) -> bool {
    if let Some(h) = editor.helper_mut() {
        h.set_ctx(CompleteCtx::default());
        h.set_prompt("", "");
    }
    matches!(editor.readline(question), Ok(answer) if is_yes(&answer))
}

/// Whether an answer to a `[y/N]` question is a yes.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
