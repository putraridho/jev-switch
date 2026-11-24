//! One key at an approval question: yes, no, always, or interrupt.

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
/// wait, the type-ahead reader for each poll ([`crate::input::typeahead`]). So a key meant for a
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

/// What a chunk of key bytes answers, for readers that get whole chunks (the Windows console):
/// Esc alone says no, an escape sequence (an arrow key) says nothing, else the first byte that
/// answers does. Same rules as the byte-at-a-time Unix reader.
#[cfg(any(windows, test))]
pub fn answer_for_chunk(chunk: &[u8], always_offered: bool) -> Option<AnswerKey> {
    match chunk.first() {
        Some(0x1b) if chunk.len() == 1 => Some(AnswerKey::No),
        Some(0x1b) => None,
        _ => chunk.iter().find_map(|b| answer_for_byte(*b, always_offered)),
    }
}

/// The Windows console: the same question, read as key events turned into bytes. Ctrl-C is a
/// key while the console is raw, so it interrupts the turn as it does on Unix.
#[cfg(windows)]
pub fn read_answer(always_offered: bool, cancel: &std::sync::atomic::AtomicBool) -> Option<AnswerKey> {
    use crate::tty::key_bytes::console_keys::{drain, poll_bytes, Polled, RawMode};
    use std::io::IsTerminal;
    use std::sync::atomic::Ordering;
    // The type-ahead reader stays off the console until the question is answered.
    let _owner = STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !std::io::stdin().is_terminal() {
        return Some(AnswerKey::No);
    }
    let Some(_raw) = RawMode::enter() else { return Some(AnswerKey::No) };
    // Anything typed before the question appeared must not answer it.
    drain();
    loop {
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        match poll_bytes(std::time::Duration::from_millis(KEY_POLL_MS as u64)) {
            Polled::Timeout => {}
            Polled::Closed => return Some(AnswerKey::No),
            Polled::Bytes(chunk) => {
                if let Some(answer) = answer_for_chunk(&chunk, always_offered) {
                    return Some(answer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_answers_like_the_keys_in_it() {
        assert_eq!(answer_for_chunk(b"y", false), Some(AnswerKey::Yes));
        assert_eq!(answer_for_chunk(b"N", false), Some(AnswerKey::No));
        assert_eq!(answer_for_chunk(b"a", true), Some(AnswerKey::Always));
        assert_eq!(answer_for_chunk(b"a", false), None, "no rule can be saved");
        assert_eq!(answer_for_chunk(&[0x03], true), Some(AnswerKey::Interrupt));
        assert_eq!(answer_for_chunk(&[0x1b], true), Some(AnswerKey::No), "Esc alone says no");
        assert_eq!(answer_for_chunk(b"\x1b[A", true), None, "an arrow key says nothing");
        assert_eq!(answer_for_chunk(b"\r", true), None, "Enter never answers");
    }
}
