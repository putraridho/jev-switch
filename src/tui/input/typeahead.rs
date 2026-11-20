//! Typing while a turn runs. A reader thread takes the keys off the terminal while the
//! model works: what is typed shows at the end of the status line, Enter queues it as the
//! next message (run when the turn is over), and Esc stops the turn; a message typed
//! before Esc is sent right after the stop ("force send"). Whatever is still typed when
//! the turn ends is put back into the prompt.
//!
//! The terminal is shared with the approval question, which reads one key itself: both
//! take [`crate::tui::input::line_editor::STDIN_LOCK`] around every read, the question for its whole
//! wait, so a key meant for a question is never taken as typing (and the reader stays
//! away while a pager owns the screen). Ctrl-C is untouched: it stays a signal.
//!
//! What a chunk of input means ([`keys`]) and what a key does to the draft
//! ([`Draft::apply`]) are pure and tested; the thread around them is thin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::tui::console::terminal_ui::Console;

/// One key, as far as the draft cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Text(String),
    /// A new line inside the message (Alt-Enter, or a line break inside pasted text).
    Newline,
    /// Enter: queue the message.
    Enter,
    Backspace,
    /// Ctrl-U.
    ClearLine,
    /// Ctrl-W.
    DeleteWord,
    /// Esc on its own: stop the turn.
    Esc,
}

/// What one chunk read from the terminal means. Escape sequences (arrows, function keys)
/// are dropped; Esc + Enter is a new line; a line break that is not the last byte of the
/// chunk is pasted text and also a new line, so a multi-line paste is one message.
pub fn keys(chunk: &[u8]) -> Vec<Key> {
    let mut out: Vec<Key> = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    let flush = |text: &mut Vec<u8>, out: &mut Vec<Key>| {
        if !text.is_empty() {
            out.push(Key::Text(String::from_utf8_lossy(text).into_owned()));
            text.clear();
        }
    };
    let mut i = 0;
    while i < chunk.len() {
        let b = chunk[i];
        match b {
            0x1b => {
                flush(&mut text, &mut out);
                i += escape_len(&chunk[i..], &mut out);
                continue;
            }
            b'\r' | b'\n' => {
                flush(&mut text, &mut out);
                // A CRLF pair is one break.
                if b == b'\r' && chunk.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                out.push(if i + 1 >= chunk.len() { Key::Enter } else { Key::Newline });
            }
            0x7f | 0x08 => {
                flush(&mut text, &mut out);
                out.push(Key::Backspace);
            }
            0x15 => {
                flush(&mut text, &mut out);
                out.push(Key::ClearLine);
            }
            0x17 => {
                flush(&mut text, &mut out);
                out.push(Key::DeleteWord);
            }
            b'\t' => text.push(b' '),
            b if b < 0x20 => {}
            _ => text.push(b),
        }
        i += 1;
    }
    flush(&mut text, &mut out);
    out
}

/// How many bytes the escape at the start of `s` takes, pushing what it means.
fn escape_len(s: &[u8], out: &mut Vec<Key>) -> usize {
    match s.get(1) {
        None => {
            out.push(Key::Esc);
            1
        }
        Some(b'\r' | b'\n') => {
            out.push(Key::Newline);
            2
        }
        // CSI: parameters, then one final byte in 0x40..=0x7e.
        Some(b'[') => 2 + s[2..].iter().position(|b| (0x40..=0x7e).contains(b)).map_or(s.len() - 2, |p| p + 1),
        // SS3: one more byte.
        Some(b'O') => s.len().min(3),
        Some(0x1b) => {
            out.push(Key::Esc);
            1
        }
        // Alt and a key: ignored.
        Some(_) => 2,
    }
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    /// The draft changed: redraw it.
    Changed,
    /// Enter: this message joins the queue.
    Queued(String),
    /// Esc: stop the turn, and send this message first when one was typed.
    Stop(Option<String>),
}

/// The message being typed during a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
}

impl Draft {
    pub fn apply(&mut self, key: Key) -> Effect {
        match key {
            Key::Text(t) => {
                self.text.push_str(&t);
                Effect::Changed
            }
            Key::Newline => {
                self.text.push('\n');
                Effect::Changed
            }
            Key::Backspace => Self::changed(self.text.pop().is_some()),
            Key::ClearLine => Self::changed(!std::mem::take(&mut self.text).is_empty()),
            Key::DeleteWord => {
                let before = self.text.len();
                let kept = self.text.trim_end().trim_end_matches(|c: char| !c.is_whitespace()).len();
                self.text.truncate(kept);
                Self::changed(kept != before)
            }
            Key::Enter => {
                if self.text.trim().is_empty() {
                    self.text.clear();
                    Effect::Nothing
                } else {
                    Effect::Queued(std::mem::take(&mut self.text).trim().to_string())
                }
            }
            Key::Esc => {
                let typed = std::mem::take(&mut self.text);
                Effect::Stop((!typed.trim().is_empty()).then(|| typed.trim().to_string()))
            }
        }
    }

    fn changed(yes: bool) -> Effect {
        if yes {
            Effect::Changed
        } else {
            Effect::Nothing
        }
    }
}

/// The reader of one turn. Dropping it (or [`TypeAhead::finish`]) stops the thread and
/// puts the terminal back.
pub struct TypeAhead {
    stop: Arc<AtomicBool>,
    esc: Arc<tokio::sync::Notify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The terminal mode from before the reader changed it, while a reader runs.
#[cfg(unix)]
static TURN_MODE: std::sync::Mutex<Option<libc::termios>> = std::sync::Mutex::new(None);

/// Put the terminal in the mode it had before the reader changed it (for a pager, which
/// expects that mode). The reader puts its own back once paging ends. Call with
/// [`crate::tui::input::line_editor::STDIN_LOCK`] held.
pub fn restore_turn_mode() {
    #[cfg(unix)]
    if let Some(saved) = *TURN_MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        // SAFETY: only changes the terminal's own state.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) };
    }
}

/// Wait (a few tens of ms at most) until no reader is in the middle of a read. Gives up
/// after a tenth of a second: the lock may be the approval reader's, held for a whole
/// question.
pub fn wait_for_reader() {
    for _ in 0..10 {
        if crate::tui::input::line_editor::STDIN_LOCK.try_lock().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// How long the reader waits for the rest of an escape sequence before taking Esc alone.
#[cfg(unix)]
const ESC_WAIT_MS: i32 = 40;

impl TypeAhead {
    /// Start reading keys for `console`'s turn. On a stdin that is not a terminal it reads
    /// nothing (and Esc never fires).
    pub fn start(console: Console) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let esc = Arc::new(tokio::sync::Notify::new());
        let thread = reader_available().then(|| {
            let (stop, esc) = (stop.clone(), esc.clone());
            std::thread::spawn(move || read_loop(&console, &stop, &esc))
        });
        Self { stop, esc, thread }
    }

    /// Resolves when Esc is pressed.
    pub async fn escaped(&self) {
        self.esc.notified().await;
    }

    /// Stop reading and wait for the terminal to be put back (one poll interval at most).
    pub fn finish(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TypeAhead {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(unix)]
fn reader_available() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(not(unix))]
fn reader_available() -> bool {
    false
}

/// Apply one chunk of input to the draft through the console; true when Esc was pressed.
#[cfg(any(test, unix))]
fn handle_chunk(console: &Console, draft: &mut Draft, chunk: &[u8]) -> bool {
    let mut esc = false;
    for key in keys(chunk) {
        match draft.apply(key) {
            Effect::Nothing => {}
            Effect::Changed => console.set_draft(&draft.text),
            Effect::Queued(message) => {
                console.set_draft("");
                console.queue_message(message, false);
            }
            Effect::Stop(message) => {
                console.set_draft("");
                if let Some(m) = message {
                    console.queue_message(m, true);
                }
                esc = true;
            }
        }
    }
    esc
}

#[cfg(unix)]
fn read_loop(console: &Console, stop: &AtomicBool, esc: &tokio::sync::Notify) {
    let fd = libc::STDIN_FILENO;
    // SAFETY: termios is plain data; tcgetattr fills it or fails.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    {
        // Read under the lock, so a question that took the terminal first (raw) is not
        // mistaken for the turn's mode.
        let _owner = crate::tui::input::line_editor::STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return;
        }
        *TURN_MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(saved);
    }
    let mut keys_mode = saved;
    // Keys one at a time and not echoed; Ctrl-C stays a signal.
    keys_mode.c_lflag &= !(libc::ICANON | libc::ECHO);
    keys_mode.c_cc[libc::VMIN] = 0;
    keys_mode.c_cc[libc::VTIME] = 0;
    let mut draft = Draft { text: console.take_draft() };
    if !draft.text.is_empty() {
        console.set_draft(&draft.text);
    }
    let pause = Duration::from_millis(crate::tui::input::line_editor::KEY_POLL_MS as u64);
    // The reader's mode is (re)applied before its next read: at the start, and after a
    // pager put the turn's mode back.
    let mut apply_mode = true;
    while !stop.load(Ordering::Acquire) {
        // A question reads its own key and a pager owns the keyboard: stay away from both.
        if console.is_paging() {
            apply_mode = true;
            std::thread::sleep(pause);
            continue;
        }
        if console.is_asking() {
            // The question (and a pager it may open) can leave another mode behind.
            apply_mode = true;
            std::thread::sleep(pause);
            continue;
        }
        let Ok(_owner) = crate::tui::input::line_editor::STDIN_LOCK.try_lock() else {
            std::thread::sleep(pause);
            continue;
        };
        // Checked again under the lock: a question may have begun meanwhile.
        if console.is_asking() || console.is_paging() {
            continue;
        }
        if apply_mode {
            // SAFETY: only changes the terminal's own state.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &keys_mode) };
            apply_mode = false;
        }
        let mut chunk = read_ready(fd, crate::tui::input::line_editor::KEY_POLL_MS);
        if chunk.last() == Some(&0x1b) {
            chunk.extend(read_ready(fd, ESC_WAIT_MS));
        }
        if !chunk.is_empty() && handle_chunk(console, &mut draft, &chunk) {
            esc.notify_one();
        }
    }
    // What is still typed goes back to the prompt.
    console.keep_draft(&draft.text);
    let _owner = crate::tui::input::line_editor::STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *TURN_MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    // SAFETY: puts back what tcgetattr read.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
}

#[cfg(not(unix))]
fn read_loop(_console: &Console, _stop: &AtomicBool, _esc: &tokio::sync::Notify) {}

/// The bytes that arrive within `wait_ms` (nothing when none do).
#[cfg(unix)]
fn read_ready(fd: i32, wait_ms: i32) -> Vec<u8> {
    let mut watched = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: polls one live descriptor struct.
    if unsafe { libc::poll(&mut watched, 1, wait_ms) } <= 0 {
        return Vec::new();
    }
    let mut buf = [0u8; 1024];
    // SAFETY: reads into a live local buffer of the given length.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n <= 0 {
        return Vec::new();
    }
    buf[..n as usize].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Key {
        Key::Text(s.into())
    }

    #[test]
    fn a_chunk_becomes_keys() {
        assert_eq!(keys(b"fix it"), [text("fix it")]);
        assert_eq!(keys(b"\r"), [Key::Enter]);
        assert_eq!(keys(b"ab\x7f"), [text("ab"), Key::Backspace]);
        assert_eq!(keys(b"\x15\x17"), [Key::ClearLine, Key::DeleteWord]);
        assert_eq!(keys("é日".as_bytes()), [text("é日")]);
    }

    #[test]
    fn escapes_are_esc_newline_or_nothing() {
        assert_eq!(keys(b"\x1b"), [Key::Esc]);
        assert_eq!(keys(b"\x1b\r"), [Key::Newline], "Alt-Enter (and Shift-Enter after /terminal-setup)");
        assert_eq!(keys(b"a\x1b[Ab"), [text("a"), text("b")], "an arrow key is dropped");
        assert_eq!(keys(b"\x1b[1;5C"), Vec::<Key>::new());
        assert_eq!(keys(b"\x1bOP"), Vec::<Key>::new(), "F1");
        assert_eq!(keys(b"\x1bx"), Vec::<Key>::new(), "Alt-x");
        assert_eq!(keys(b"\x1b\x1b"), [Key::Esc, Key::Esc]);
    }

    #[test]
    fn a_pasted_block_is_one_message() {
        assert_eq!(keys(b"one\ntwo\n"), [text("one"), Key::Newline, text("two"), Key::Enter]);
        assert_eq!(keys(b"one\r\ntwo"), [text("one"), Key::Newline, text("two")]);
    }

    #[test]
    fn enter_queues_and_esc_stops_with_what_was_typed() {
        let mut d = Draft::default();
        assert_eq!(d.apply(Key::Enter), Effect::Nothing, "an empty Enter queues nothing");
        assert_eq!(d.apply(text("run the tests ")), Effect::Changed);
        assert_eq!(d.apply(Key::Enter), Effect::Queued("run the tests".into()));
        assert_eq!(d.text, "");
        assert_eq!(d.apply(Key::Esc), Effect::Stop(None));
        d.apply(text("do this instead"));
        assert_eq!(d.apply(Key::Esc), Effect::Stop(Some("do this instead".into())));
    }

    #[test]
    fn editing_keys_change_the_draft() {
        let mut d = Draft { text: "fix the  bug".into() };
        assert_eq!(d.apply(Key::DeleteWord), Effect::Changed);
        assert_eq!(d.text, "fix the  ");
        d.apply(Key::DeleteWord);
        assert_eq!(d.text, "fix ");
        d.apply(Key::Backspace);
        assert_eq!(d.text, "fix");
        assert_eq!(d.apply(Key::ClearLine), Effect::Changed);
        assert_eq!(d.apply(Key::ClearLine), Effect::Nothing);
        assert_eq!(d.apply(Key::Backspace), Effect::Nothing);
    }

    #[test]
    fn a_chunk_through_the_console_queues_and_reports_esc() {
        use crate::testkit::capture::Buf;
        use crate::tui::console::terminal_ui::ConsoleConfig;
        let console = Console::new(
            Box::new(Buf::default()),
            Box::new(Buf::default()),
            ConsoleConfig { out_style: crate::tui::render::Style::off(), err_style: crate::tui::render::Style::off(), width: 80, live: false, project: "/tmp".into(), stream_answers: true },
        );
        let mut d = Draft::default();
        assert!(!handle_chunk(&console, &mut d, b"first\r"));
        assert!(!handle_chunk(&console, &mut d, b"urgent"));
        assert!(handle_chunk(&console, &mut d, b"\x1b"));
        assert_eq!(console.take_queued(), Some("urgent".to_string()), "a message sent with Esc goes first");
        assert_eq!(console.take_queued(), Some("first".to_string()));
        assert_eq!(console.take_queued(), None);
    }
}
