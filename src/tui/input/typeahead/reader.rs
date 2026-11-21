//! The reader thread: takes keys off the terminal during a turn and applies them to the draft.

use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::atomic::Ordering;
#[cfg(unix)]
use std::time::Duration;

use super::draft::{Draft, Effect};
use super::keys::keys;
#[cfg(unix)]
use super::TURN_MODE;
use crate::tui::console::terminal_ui::Console;

/// How long the reader waits for the rest of an escape sequence before taking Esc alone.
#[cfg(unix)]
const ESC_WAIT_MS: i32 = 40;

/// Apply one chunk of input to the draft through the console; true when Esc was pressed.
#[cfg(any(test, unix))]
pub(super) fn handle_chunk(console: &Console, draft: &mut Draft, chunk: &[u8]) -> bool {
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
pub(super) fn read_loop(console: &Console, stop: &AtomicBool, esc: &tokio::sync::Notify) {
    let fd = libc::STDIN_FILENO;
    // SAFETY: termios is plain data; tcgetattr fills it or fails.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    {
        // Read under the lock, so a question that took the terminal first (raw) is not
        // mistaken for the turn's mode.
        let _owner = crate::tui::input::line_editor::approval::STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let pause = Duration::from_millis(crate::tui::input::line_editor::approval::KEY_POLL_MS as u64);
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
        let Ok(_owner) = crate::tui::input::line_editor::approval::STDIN_LOCK.try_lock() else {
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
        let mut chunk = read_ready(fd, crate::tui::input::line_editor::approval::KEY_POLL_MS);
        if chunk.last() == Some(&0x1b) {
            chunk.extend(read_ready(fd, ESC_WAIT_MS));
        }
        if !chunk.is_empty() && handle_chunk(console, &mut draft, &chunk) {
            esc.notify_one();
        }
    }
    // What is still typed goes back to the prompt.
    console.keep_draft(&draft.text);
    let _owner = crate::tui::input::line_editor::approval::STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *TURN_MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    // SAFETY: puts back what tcgetattr read.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
}

#[cfg(not(unix))]
pub(super) fn read_loop(_console: &Console, _stop: &AtomicBool, _esc: &tokio::sync::Notify) {}

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
