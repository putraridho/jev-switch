use super::Key;

/// The keys in one read from the terminal: an escape sequence arrives whole, typed or
/// pasted text arrives as many characters.
pub(super) fn decode(bytes: &[u8]) -> Vec<Key> {
    match bytes {
        [] => vec![],
        [0x1b] => vec![Key::Cancel],
        [0x1b, b'[' | b'O', b'A'] => vec![Key::Up],
        [0x1b, b'[' | b'O', b'B'] => vec![Key::Down],
        [0x1b, b'[' | b'O', b'C'] => vec![Key::Right],
        [0x1b, b'[' | b'O', b'D'] => vec![Key::Left],
        [0x1b, b'[', b'5', b'~'] => vec![Key::PageUp],
        [0x1b, b'[', b'6', b'~'] => vec![Key::PageDown],
        // Any other escape sequence (Home, End, F-keys, ...) says nothing.
        [0x1b, ..] => vec![],
        _ => String::from_utf8_lossy(bytes)
            .chars()
            .filter_map(key_for_char)
            .collect(),
    }
}

fn key_for_char(c: char) -> Option<Key> {
    match c {
        '\r' | '\n' => Some(Key::Enter),
        '\u{7f}' | '\u{8}' => Some(Key::Backspace),
        '\u{3}' => Some(Key::Interrupt),
        '\u{4}' => Some(Key::Cancel),
        '\u{10}' => Some(Key::Up),
        '\u{e}' => Some(Key::Down),
        c if c.is_control() => None,
        c => Some(Key::Char(c)),
    }
}
