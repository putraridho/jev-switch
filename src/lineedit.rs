//! The line editor of the interactive session: history that survives restarts, Tab
//! completion and an inline hint from the command table in [`crate::uifmt`], a prompt that
//! shows the routing state, Alt-Enter for a new line, and the "Ctrl-C twice to exit" rule.
//!
//! The decisions (what completes, what the hint says, whether this Ctrl-C exits) are pure
//! and tested; the [`rustyline`] glue around them is thin.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::{DefaultHistory, History};
use rustyline::validate::Validator;
use rustyline::{Cmd, ColorMode, CompletionType, Config, ConditionalEventHandler, Context, Editor, EventContext, EventHandler, Helper, KeyCode, KeyEvent, Modifiers, Movement, RepeatCount};

use crate::mode::SharedMode;

use crate::uifmt::{self, CompleteCtx, Style};

/// Lines kept in the history file.
pub const HISTORY_MAX: usize = 2000;

/// A second Ctrl-C within this long of the first exits.
pub const INTERRUPT_WINDOW: Duration = Duration::from_secs(2);

/// What the first Ctrl-C at the prompt says.
pub const EXIT_HINT: &str = "(press Ctrl-C again or Ctrl-D to exit)";

// ------------------------------------------------------------------- Ctrl-C rule

/// What a Ctrl-C at the prompt should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    /// The first one (or one after a pause): clear the line and say how to exit.
    Warn,
    /// A second one within [`INTERRUPT_WINDOW`]: exit.
    Exit,
}

/// Remembers when the last Ctrl-C at the prompt was. Time is passed in, so the rule is a
/// plain unit test.
#[derive(Debug, Default)]
pub struct InterruptGuard {
    last: Option<Instant>,
}

impl InterruptGuard {
    pub fn on_interrupt(&mut self, now: Instant) -> Interrupt {
        let exit = self.last.is_some_and(|t| now.saturating_duration_since(t) <= INTERRUPT_WINDOW);
        self.last = Some(now);
        if exit {
            Interrupt::Exit
        } else {
            Interrupt::Warn
        }
    }

    /// Anything the user typed and sent since: the next Ctrl-C is a first one again.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

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
pub fn mode_hint(mode: crate::mode::Mode) -> String {
    format!("[{mode} mode \u{b7} shift-tab to change]")
}

/// Shift-Tab: the next mode, shown at once.
struct CycleMode(SharedMode);

impl ConditionalEventHandler for CycleMode {
    fn handle(&self, _: &rustyline::Event, _: RepeatCount, _: bool, _: &EventContext) -> Option<Cmd> {
        self.0.cycle();
        Some(Cmd::Repaint)
    }
}

/// Enter right after a `\\` at the end of the line: the backslash becomes a new line, as
/// in a shell. Anywhere else Enter sends the line.
struct BackslashNewline;

impl ConditionalEventHandler for BackslashNewline {
    fn handle(&self, _: &rustyline::Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        continues_line(ctx.line(), ctx.pos()).then(|| Cmd::Replace(Movement::BackwardChar(1), Some("\n".to_string())))
    }
}

/// Whether Enter at `pos` continues the line: the cursor is at the end, after one `\\`
/// (a doubled one is a literal backslash).
pub fn continues_line(line: &str, pos: usize) -> bool {
    pos == line.len() && line.ends_with('\\') && !line.ends_with("\\\\")
}

/// Most `@path` candidates Tab lists at once.
const MAX_PATH_CANDIDATES: usize = 200;

/// Candidates for an `@path` word (given without the `@`): the entries of its folder that
/// start with what was typed, folders with a `/`; and, for a word with no `/` yet, the
/// tracked files anywhere whose name starts with it (`@main` finds `src/main.rs`). Hidden
/// entries only when typed with a dot; `.git` never. Spaces are escaped, as a pasted
/// path would be.
pub fn path_candidates(word: &str, project: &Path, tracked: &[String]) -> Vec<Pair> {
    let (folder, prefix) = word.rsplit_once('/').map_or(("", word), |(d, p)| (d, p));
    let lower = prefix.to_lowercase();
    let mut found: Vec<(String, bool)> = Vec::new();
    let base = if folder.is_empty() { project.to_path_buf() } else { project.join(folder) };
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let hidden_ok = !name.starts_with('.') || prefix.starts_with('.');
            if name == ".git" || !hidden_ok || !name.to_lowercase().starts_with(&lower) {
                continue;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let path = if folder.is_empty() { name } else { format!("{folder}/{name}") };
            found.push((path, is_dir));
        }
    }
    found.sort();
    if !word.contains('/') && !word.is_empty() {
        let deeper = tracked.iter().filter(|t| t.contains('/') && t.rsplit('/').next().is_some_and(|n| n.to_lowercase().starts_with(&lower)));
        found.extend(deeper.map(|t| (t.clone(), false)));
    }
    found.dedup();
    found
        .into_iter()
        .take(MAX_PATH_CANDIDATES)
        .map(|(path, is_dir)| {
            let shown = if is_dir { format!("{path}/") } else { path };
            Pair { display: shown.clone(), replacement: format!("@{}", shown.replace(' ', "\\ ")) }
        })
        .collect()
}

/// The files git tracks in `dir` (empty outside a repository).
pub fn tracked_files(dir: &Path) -> Vec<String> {
    let out = std::process::Command::new("git").args(["ls-files", "-z"]).current_dir(dir).stderr(std::process::Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// The completions of the word before the cursor: where that word starts, and the
/// candidates. Pure, so Tab behaviour is tested without a terminal.
pub fn completions_for(line: &str, pos: usize, ctx: &CompleteCtx) -> (usize, Vec<Pair>) {
    let upto = line.get(..pos).unwrap_or(line);
    let candidates = uifmt::complete(upto, ctx)
        .into_iter()
        .map(|c| Pair { display: c.display, replacement: c.replacement })
        .collect();
    (uifmt::completion_start(upto), candidates)
}

impl Completer for JevHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let upto = line.get(..pos).unwrap_or(line);
        let start = uifmt::completion_start(upto);
        if let (Some(word), Some(project)) = (upto[start..].strip_prefix('@'), &self.project) {
            return Ok((start, path_candidates(word, project, &tracked_files(project))));
        }
        Ok(completions_for(line, pos, &self.ctx))
    }
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

// --------------------------------------------------------------------- history

/// `<state dir>/history`.
pub fn history_path(state_dir: &Path) -> PathBuf {
    state_dir.join("history")
}

/// Create the history file readable by its owner only, before anything is written to it:
/// prompts can be private.
pub fn prepare_history_file(path: &Path) {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let _ = opts.open(path);
    restrict(path);
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

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

/// Add `line` to the history and write it to the file. Blank lines are not kept.
pub fn remember(editor: &mut LineEditor, line: &str, history: Option<&Path>) {
    if line.trim().is_empty() {
        return;
    }
    if editor.add_history_entry(line).unwrap_or(false) {
        if let Some(path) = history {
            let _ = editor.append_history(path);
            restrict(path);
        }
    }
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

/// How many entries the editor remembers (for tests and `/status`-style checks).
pub fn history_len(editor: &LineEditor) -> usize {
    editor.history().len()
}

// ------------------------------------------------------------ one key: approvals

/// The answer to a "may this tool run?" question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKey {
    Yes,
    No,
    /// Yes, and remember it for this project.
    Always,
    /// Ctrl-C: stop the turn.
    Interrupt,
}

/// Whoever reads keys from the terminal holds this: the approval question for its whole
/// wait, the type-ahead reader for each poll ([`crate::typeahead`]). So a key meant for a
/// question is never taken as typing.
pub static STDIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long the wait for an approval key goes without looking at its cancel flag; also the
/// longest a cancelled read takes to stop.
pub const KEY_POLL_MS: i32 = 20;

/// What one byte typed at the question means. `None` keeps waiting: Enter and stray
/// letters never answer by accident, and `a` only counts when a rule could be saved.
pub fn answer_for_byte(byte: u8, always_offered: bool) -> Option<AnswerKey> {
    match byte {
        b'y' | b'Y' => Some(AnswerKey::Yes),
        b'n' | b'N' | 0x04 => Some(AnswerKey::No),
        b'a' | b'A' if always_offered => Some(AnswerKey::Always),
        0x03 => Some(AnswerKey::Interrupt),
        _ => None,
    }
}

/// Wait for one key at an approval question: no Enter, no echo. Anything typed before the
/// question appeared is thrown away first, so a stray `y` typed ahead cannot approve a call
/// the person never saw. Esc alone says no; an arrow key (Esc and more) says nothing. A
/// stdin that is not a terminal, or is closed, says no at once: it must never hang.
///
/// The wait looks at `cancel` every [`KEY_POLL_MS`]; once it is set the read gives up with
/// `None`, having put the terminal back as it found it and read nothing: a key typed after
/// that stays in the terminal's queue for whoever reads next.
#[cfg(unix)]
pub fn read_answer(always_offered: bool, cancel: &std::sync::atomic::AtomicBool) -> Option<AnswerKey> {
    use std::sync::atomic::Ordering;

    /// True when a key can be read; false when `cancel` was set first (it wins over a key
    /// that arrives at the same moment, which is left unread).
    fn key_ready(fd: i32, cancel: &std::sync::atomic::AtomicBool) -> bool {
        loop {
            if cancel.load(Ordering::Acquire) {
                return false;
            }
            let mut watched = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: polls one live descriptor struct.
            let n = unsafe { libc::poll(&mut watched, 1, KEY_POLL_MS) };
            if n > 0 {
                return !cancel.load(Ordering::Acquire);
            }
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                // Let the read report what is wrong.
                return true;
            }
        }
    }
    fn read_byte(fd: i32) -> Option<u8> {
        let mut byte = 0u8;
        loop {
            // SAFETY: reads at most one byte into a live local.
            let n = unsafe { libc::read(fd, std::ptr::addr_of_mut!(byte).cast(), 1) };
            if n == 1 {
                return Some(byte);
            }
            if n == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return None;
            }
        }
    }
    let fd = libc::STDIN_FILENO;
    // The type-ahead reader stays off the terminal until the question is answered.
    let _owner = STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: termios is plain data; tcgetattr fills it or fails.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return Some(AnswerKey::No);
    }
    let mut raw = saved;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    // SAFETY: both calls only change the terminal's own state.
    unsafe {
        libc::tcflush(fd, libc::TCIFLUSH);
        libc::tcsetattr(fd, libc::TCSANOW, &raw);
    }
    let answer = loop {
        if !key_ready(fd, cancel) {
            break None;
        }
        let Some(byte) = read_byte(fd) else { break Some(AnswerKey::No) };
        if byte == 0x1b {
            // Wait a tenth of a second for the rest of an escape sequence.
            let mut quick = raw;
            quick.c_cc[libc::VMIN] = 0;
            quick.c_cc[libc::VTIME] = 1;
            // SAFETY: as above.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quick) };
            let mut sequence = false;
            while read_byte(fd).is_some() {
                sequence = true;
            }
            // SAFETY: as above.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
            if sequence {
                continue;
            }
            break Some(AnswerKey::No);
        }
        if let Some(answer) = answer_for_byte(byte, always_offered) {
            break Some(answer);
        }
    };
    // SAFETY: puts back what tcgetattr read.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    answer
}

/// Without a terminal API there is no key to wait for: say no.
#[cfg(not(unix))]
pub fn read_answer(_always_offered: bool, _cancel: &std::sync::atomic::AtomicBool) -> Option<AnswerKey> {
    Some(AnswerKey::No)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_keys_answer_only_when_they_mean_it() {
        use AnswerKey::*;
        for (byte, offered, want) in [
            (b'y', true, Some(Yes)),
            (b'Y', false, Some(Yes)),
            (b'n', true, Some(No)),
            (0x04, true, Some(No)),
            (b'a', true, Some(Always)),
            (b'A', true, Some(Always)),
            (b'a', false, None),
            (0x03, false, Some(Interrupt)),
            (b'\r', true, None),
            (b'\n', true, None),
            (b' ', true, None),
            (b'q', true, None),
            (b'[', true, None),
        ] {
            assert_eq!(answer_for_byte(byte, offered), want, "{byte:#x} offered={offered}");
        }
    }

    /// With no terminal on stdin (as under `cargo test`) the question answers no and returns.
    #[cfg(unix)]
    #[test]
    fn reading_a_key_without_a_terminal_never_waits() {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            assert_eq!(read_answer(true, &std::sync::atomic::AtomicBool::new(false)), Some(AnswerKey::No));
        }
    }

    #[test]
    fn a_second_ctrl_c_within_two_seconds_exits() {
        let mut g = InterruptGuard::default();
        let t0 = Instant::now();
        assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
        assert_eq!(g.on_interrupt(t0 + Duration::from_millis(1900)), Interrupt::Exit);
    }

    #[test]
    fn a_pause_longer_than_the_window_starts_over() {
        let mut g = InterruptGuard::default();
        let t0 = Instant::now();
        assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
        assert_eq!(g.on_interrupt(t0 + Duration::from_millis(2500)), Interrupt::Warn);
        assert_eq!(g.on_interrupt(t0 + Duration::from_millis(3000)), Interrupt::Exit, "the window counts from the latest one");
    }

    #[test]
    fn sending_a_message_between_two_ctrl_cs_resets_the_rule() {
        let mut g = InterruptGuard::default();
        let t0 = Instant::now();
        assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
        g.reset();
        assert_eq!(g.on_interrupt(t0 + Duration::from_millis(100)), Interrupt::Warn);
    }

    fn ctx() -> CompleteCtx {
        CompleteCtx {
            ready_providers: vec!["claude".into(), "codex".into()],
            models: vec!["codex:gpt-6.1-sol".into()],
            changed_files: vec!["src/a.rs".into()],
            skills: Vec::new(),
        }
    }

    #[test]
    fn tab_completes_commands_and_their_arguments() {
        let (start, c) = completions_for("/mode", 5, &ctx());
        assert_eq!(start, 0);
        assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["/mode", "/model"]);

        let (start, c) = completions_for("/model co", 9, &ctx());
        assert_eq!(start, 7, "only the word being completed is replaced");
        assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["codex", "codex:gpt-6.1-sol"]);

        let (_, c) = completions_for("/diff ", 6, &ctx());
        assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["src/a.rs"]);
    }

    #[test]
    fn enter_after_one_backslash_continues_the_line() {
        assert!(continues_line("first line\\", 11));
        assert!(!continues_line("first line\\", 3), "only at the end of the line");
        assert!(!continues_line("a literal \\\\", 12), "a doubled backslash is literal");
        assert!(!continues_line("plain", 5));
    }

    #[test]
    fn at_paths_complete_from_the_folder_and_tracked_files() {
        let dir = temp("atpath");
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("my notes.md"), "").unwrap();
        std::fs::write(dir.join(".env"), "").unwrap();
        let names = |word: &str, tracked: &[String]| path_candidates(word, &dir, tracked).into_iter().map(|p| p.replacement).collect::<Vec<_>>();
        assert_eq!(names("", &[]), ["@my\\ notes.md", "@src/"]);
        assert_eq!(names("s", &[]), ["@src/"]);
        assert_eq!(names("src/", &[]), ["@src/deep/", "@src/main.rs"]);
        assert_eq!(names("src/M", &[]), ["@src/main.rs"], "case is ignored");
        assert_eq!(names(".", &[]), ["@.env"], "hidden only when asked, never .git");
        assert_eq!(names("main", &["src/main.rs".to_string(), "README.md".to_string()]), ["@src/main.rs"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prose_and_a_cursor_in_the_middle_are_handled() {
        assert!(completions_for("fix the bug", 11, &ctx()).1.is_empty());
        // The cursor sits after "/mo": the rest of the line is not part of the word.
        let (start, c) = completions_for("/model auto", 3, &ctx());
        assert_eq!(start, 0);
        assert!(c.iter().all(|p| p.replacement.starts_with("/mo")));
        // A position past the end (or inside a multi-byte char) never panics.
        let _ = completions_for("é/mod", 99, &ctx());
    }

    #[test]
    fn the_hint_is_dim_only_when_styled() {
        let h = JevHelper::new(Style::off(), SharedMode::default());
        assert_eq!(h.highlight_hint("el"), "el");
        assert_eq!(JevHelper::new(Style::on(), SharedMode::default()).highlight_hint("el"), "\x1b[2mel\x1b[0m");
    }

    #[test]
    fn an_empty_line_hints_the_mode_and_a_typed_one_does_not() {
        use crate::mode::Mode;
        let h = JevHelper::new(Style::off(), SharedMode::new(Mode::Auto));
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        assert_eq!(h.hint("", 0, &ctx).as_deref(), Some("[auto mode \u{b7} shift-tab to change]"));
        assert_eq!(h.hint("hello", 5, &ctx), None);
    }

    #[test]
    fn the_prompt_is_coloured_only_when_it_is_ours_and_colour_is_on() {
        let mut h = JevHelper::new(Style::on(), SharedMode::default());
        h.set_prompt("› ", "\x1b[36m›\x1b[0m ");
        assert_eq!(h.highlight_prompt("› ", true), "\x1b[36m›\x1b[0m ");
        assert_eq!(h.highlight_prompt("› ", false), "› ", "an incremental-search prompt stays plain");
        assert_eq!(h.highlight_prompt("(reverse-i-search)`': ", true), "(reverse-i-search)`': ");
        let mut off = JevHelper::new(Style::off(), SharedMode::default());
        off.set_prompt("› ", "x");
        assert_eq!(off.highlight_prompt("› ", true), "› ");
    }

    #[test]
    fn yes_means_y_or_yes_and_nothing_else() {
        for yes in ["y", "Y", "yes", " YES "] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "n", "no", "yep", "ye", "1"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-lineedit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn history_is_a_private_file_that_remembers_lines_across_editors() {
        let dir = temp("history");
        let path = history_path(&dir);
        let mut first = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
        remember(&mut first, "fix the bug", Some(&path));
        remember(&mut first, "   ", Some(&path));
        remember(&mut first, "fix the bug", Some(&path));
        remember(&mut first, "/model codex", Some(&path));
        assert_eq!(history_len(&first), 2, "blank lines and consecutive duplicates are not kept");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let second = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
        let last: Vec<String> = second.history().iter().cloned().collect();
        assert_eq!(last, ["fix the bug", "/model codex"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn multi_line_messages_survive_in_history() {
        let dir = temp("multiline");
        let path = history_path(&dir);
        let mut first = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
        remember(&mut first, "line one\nline two", Some(&path));
        let second = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
        assert_eq!(second.history().iter().next().map(String::as_str), Some("line one\nline two"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_history_is_capped() {
        let dir = temp("cap");
        let path = history_path(&dir);
        let mut e = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
        for i in 0..HISTORY_MAX + 10 {
            remember(&mut e, &format!("message {i}"), Some(&path));
        }
        assert_eq!(history_len(&e), HISTORY_MAX);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failure_of_the_editor_is_described_in_words() {
        let e = ReadlineError::Io(std::io::Error::other("boom"));
        assert_eq!(describe(&e), "the terminal failed: boom");
    }
}
