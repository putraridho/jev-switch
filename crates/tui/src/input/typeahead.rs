//! Typing while a turn runs. A reader thread takes the keys off the terminal while the
//! model works: what is typed shows at the end of the status line, Enter queues it as the
//! next message (run when the turn is over), and Esc stops the turn; a message typed
//! before Esc is sent right after the stop ("force send"). Whatever is still typed when
//! the turn ends is put back into the prompt.
//!
//! The terminal is shared with the approval question, which reads one key itself: both
//! take [`crate::input::line_editor::approval::STDIN_LOCK`] around every read, the question for its whole
//! wait, so a key meant for a question is never taken as typing (and the reader stays
//! away while a pager owns the screen). Ctrl-C is untouched: it stays a signal.
//!
//! What a chunk of input means ([`keys::keys`]) and what a key does to the draft
//! ([`draft::Draft::apply`]) are pure and tested; the thread around them is thin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::console::terminal_ui::Console;
use reader::read_loop;

mod draft;
mod keys;
mod reader;
#[cfg(test)]
mod tests;

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
/// [`crate::input::line_editor::approval::STDIN_LOCK`] held.
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
        if crate::input::line_editor::approval::STDIN_LOCK.try_lock().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

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
