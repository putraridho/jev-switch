//! What a chunk of input read from the terminal means: keys, with escape sequences dropped.

/// One key, as far as the draft cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
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
pub(crate) fn keys(chunk: &[u8]) -> Vec<Key> {
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
