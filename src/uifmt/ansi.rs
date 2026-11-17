//! ANSI escape sequences: telling them from printable text, stripping them, and the
//! few cursor-control strings the status line needs.

/// The SGR reset.
pub(super) const RESET: &str = "\x1b[0m";

/// One piece of text: an escape sequence (zero width) or a printable char.
pub(super) enum Seg<'a> {
    Esc(&'a str),
    Ch(char),
}

pub(super) struct Segs<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Segs<'a> {
    type Item = Seg<'a>;

    fn next(&mut self) -> Option<Seg<'a>> {
        let c = self.rest.chars().next()?;
        if c == '\x1b' {
            let n = escape_len(self.rest);
            let (esc, rest) = self.rest.split_at(n);
            self.rest = rest;
            Some(Seg::Esc(esc))
        } else {
            self.rest = &self.rest[c.len_utf8()..];
            Some(Seg::Ch(c))
        }
    }
}

pub(super) fn segs(s: &str) -> Segs<'_> {
    Segs { rest: s }
}

/// Byte length of the escape sequence at the start of `s` (which starts with ESC).
/// Handles CSI (`ESC [ ... final`), OSC (`ESC ] ... BEL|ST`) and two-byte escapes
/// (`ESC 7`). A malformed or unterminated sequence ends where it stops making sense,
/// always on a char boundary.
fn escape_len(s: &str) -> usize {
    let b = s.as_bytes();
    match b.get(1) {
        None => 1,
        Some(b'[') => {
            let mut i = 2;
            while i < b.len() {
                match b[i] {
                    0x20..=0x3f => i += 1,
                    0x40..=0x7e => return i + 1,
                    _ => return i,
                }
            }
            b.len()
        }
        Some(b']') => {
            let mut i = 2;
            while i < b.len() {
                match b[i] {
                    0x07 => return i + 1,
                    0x1b if b.get(i + 1) == Some(&b'\\') => return i + 2,
                    _ => i += 1,
                }
            }
            b.len()
        }
        Some(_) => 1 + s[1..].chars().next().map_or(0, char::len_utf8),
    }
}

/// `s` without ANSI escape sequences.
pub fn strip_ansi(s: &str) -> String {
    segs(s)
        .filter_map(|seg| match seg {
            Seg::Ch(c) => Some(c),
            Seg::Esc(_) => None,
        })
        .collect()
}

/// Erase the current line and return to column 0.
pub fn clear_line() -> &'static str {
    "\r\x1b[2K"
}

/// Move the cursor up one row, keeping its column.
pub fn up_row() -> &'static str {
    "\x1b[1A"
}

pub fn save_cursor() -> &'static str {
    "\x1b7"
}

pub fn restore_cursor() -> &'static str {
    "\x1b8"
}

pub fn hide_cursor() -> &'static str {
    "\x1b[?25l"
}

pub fn show_cursor() -> &'static str {
    "\x1b[?25h"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::display_width;

    #[test]
    fn cursor_escapes() {
        assert_eq!(clear_line(), "\r\x1b[2K");
        assert_eq!(save_cursor(), "\x1b7");
        assert_eq!(restore_cursor(), "\x1b8");
        assert_eq!(hide_cursor(), "\x1b[?25l");
        assert_eq!(show_cursor(), "\x1b[?25h");
        // they take no columns
        assert_eq!(display_width(&format!("{}x{}", save_cursor(), restore_cursor())), 1);
        assert_eq!(display_width(&format!("{}x{}", hide_cursor(), show_cursor())), 1);
    }
}
