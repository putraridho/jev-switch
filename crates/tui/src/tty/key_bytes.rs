//! Keys as the bytes a terminal sends for them.
//!
//! Everything that reads the keyboard outside the full-screen front end (the approval question,
//! type-ahead during a turn, the pickers) works on the bytes a Unix terminal sends: `\r`,
//! `\x7f`, `\x1b[A`. The Windows console gives key *events* instead, so [`key_bytes`] turns an
//! event into the bytes the same code already understands. [`console_keys`] (Windows only)
//! reads them.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The bytes a terminal would send for one key press. A release sends nothing, and neither do
/// keys nothing here reads (media keys, a bare modifier).
pub fn key_bytes(event: &KeyEvent) -> Vec<u8> {
    if event.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    let shift = event.modifiers.contains(KeyModifiers::SHIFT);
    let mut bytes = match event.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift-Enter and Alt-Enter are a new line inside the message: Esc, Enter.
        KeyCode::Enter if shift || alt => return b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(_) => b"\x1bOP".to_vec(),
        _ => return Vec::new(),
    };
    // Alt and a letter is Esc, then the letter.
    if alt && matches!(event.code, KeyCode::Char(_)) {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Reading keys from the Windows console as bytes.
#[cfg(windows)]
pub mod console_keys {
    use super::key_bytes;
    use crossterm::event::{poll, read, Event};
    use std::sync::Mutex;
    use std::time::Duration;

    /// What one wait for keys gave.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Polled {
        Bytes(Vec<u8>),
        Timeout,
        /// The console is gone or cannot be read.
        Closed,
    }

    /// A lone Esc that came after other keys in one batch, kept for the next read so it is not
    /// glued to the keys after it (Esc then `x` would read as Alt-x).
    static CARRY: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    /// Wait up to `timeout` for keys. Keys that are already queued (a paste) come together, so
    /// a multi-line paste stays one chunk.
    pub fn poll_bytes(timeout: Duration) -> Polled {
        {
            let mut carry = CARRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !carry.is_empty() {
                return Polled::Bytes(std::mem::take(&mut *carry));
            }
        }
        match poll(timeout) {
            Ok(false) => return Polled::Timeout,
            Err(_) => return Polled::Closed,
            Ok(true) => {}
        }
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match read() {
                Ok(Event::Key(key)) => {
                    let next = key_bytes(&key);
                    if next == [0x1b] && !bytes.is_empty() {
                        *CARRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = next;
                        break;
                    }
                    bytes.extend(next);
                    if bytes == [0x1b] {
                        break;
                    }
                }
                Ok(Event::Paste(text)) => bytes.extend(text.into_bytes()),
                Ok(_) => {}
                Err(_) => return Polled::Closed,
            }
            if !matches!(poll(Duration::ZERO), Ok(true)) {
                break;
            }
        }
        if bytes.is_empty() {
            Polled::Timeout
        } else {
            Polled::Bytes(bytes)
        }
    }

    /// Throw away everything typed so far.
    pub fn drain() {
        CARRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        while matches!(poll(Duration::ZERO), Ok(true)) {
            if read().is_err() {
                break;
            }
        }
    }

    /// The console in raw mode (no line buffering, no echo, Ctrl-C a key) while this lives.
    /// Left as it was found when dropped, and untouched when it was already raw.
    pub struct RawMode {
        enabled_here: bool,
    }

    impl RawMode {
        pub fn enter() -> Option<RawMode> {
            let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if !was_raw {
                crossterm::terminal::enable_raw_mode().ok()?;
            }
            Some(RawMode { enabled_here: !was_raw })
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            if self.enabled_here {
                let _ = crossterm::terminal::disable_raw_mode();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        key_bytes(&KeyEvent::new(code, modifiers))
    }

    #[test]
    fn plain_keys_are_what_a_unix_terminal_sends() {
        assert_eq!(press(KeyCode::Char('y'), KeyModifiers::NONE), b"y");
        assert_eq!(press(KeyCode::Char('Y'), KeyModifiers::SHIFT), b"Y");
        assert_eq!(press(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
        assert_eq!(press(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(press(KeyCode::Backspace, KeyModifiers::NONE), [0x7f]);
        assert_eq!(press(KeyCode::Esc, KeyModifiers::NONE), [0x1b]);
        assert_eq!(press(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
    }

    #[test]
    fn control_letters_are_control_bytes() {
        assert_eq!(press(KeyCode::Char('c'), KeyModifiers::CONTROL), [0x03], "Ctrl-C interrupts an approval question");
        assert_eq!(press(KeyCode::Char('D'), KeyModifiers::CONTROL), [0x04]);
        assert_eq!(press(KeyCode::Char('u'), KeyModifiers::CONTROL), [0x15]);
        assert_eq!(press(KeyCode::Char('w'), KeyModifiers::CONTROL), [0x17]);
    }

    #[test]
    fn shift_or_alt_enter_is_a_new_line_and_alt_letters_are_esc_prefixed() {
        assert_eq!(press(KeyCode::Enter, KeyModifiers::SHIFT), b"\x1b\r");
        assert_eq!(press(KeyCode::Enter, KeyModifiers::ALT), b"\x1b\r");
        assert_eq!(press(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
    }

    #[test]
    fn releases_and_unread_keys_send_nothing() {
        let mut release = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(key_bytes(&release).is_empty());
        assert!(press(KeyCode::CapsLock, KeyModifiers::NONE).is_empty());
    }

    #[test]
    fn these_bytes_read_back_as_the_same_keys_in_the_draft() {
        use crate::input::typeahead::keys::{keys, Key};
        assert_eq!(keys(&press(KeyCode::Esc, KeyModifiers::NONE)), vec![Key::Esc]);
        assert_eq!(keys(&press(KeyCode::Enter, KeyModifiers::NONE)), vec![Key::Enter]);
        assert_eq!(keys(&press(KeyCode::Enter, KeyModifiers::SHIFT)), vec![Key::Newline]);
        assert_eq!(keys(&press(KeyCode::Left, KeyModifiers::NONE)), Vec::<Key>::new(), "arrows are dropped, as on Unix");
        assert_eq!(keys(&press(KeyCode::Char('a'), KeyModifiers::NONE)), vec![Key::Text("a".into())]);
    }
}
