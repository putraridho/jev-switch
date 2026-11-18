//! The text in the input box and the cursor in it: plain data and pure edits, so every key's
//! effect is a unit test. Lines are separated by `\n`; the cursor is a byte offset that always
//! sits on a character boundary.

use crate::uifmt::{char_width, display_width};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Editor {
    text: String,
    cursor: usize,
}

/// Where a word starts or ends: letters, digits and `_` make words; anything else separates.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text up to the cursor.
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    /// Replace everything, cursor at the end.
    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
    }

    /// Take the text out, leaving the box empty.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    /// Replace the bytes `from..cursor` with `with`, cursor after it (completion).
    pub fn replace_before_cursor(&mut self, from: usize, with: &str) {
        let from = from.min(self.cursor);
        self.text.replace_range(from..self.cursor, with);
        self.cursor = from + with.len();
    }

    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert pasted text as it is, with its line breaks normalised to `\n` and tabs to four
    /// spaces; other control characters are dropped.
    pub fn insert_str(&mut self, s: &str) {
        let clean: String = s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor].char_indices().next_back().map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..].chars().next().map(|c| self.cursor + c.len_utf8())
    }

    pub fn backspace(&mut self) {
        if let Some(i) = self.prev_boundary() {
            self.text.replace_range(i..self.cursor, "");
            self.cursor = i;
        }
    }

    pub fn delete(&mut self) {
        if let Some(j) = self.next_boundary() {
            self.text.replace_range(self.cursor..j, "");
        }
    }

    pub fn left(&mut self) {
        if let Some(i) = self.prev_boundary() {
            self.cursor = i;
        }
    }

    pub fn right(&mut self) {
        if let Some(j) = self.next_boundary() {
            self.cursor = j;
        }
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i)
    }

    /// To the start of the line (Home, Ctrl-A).
    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// To the end of the line (End, Ctrl-E).
    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    fn word_start_before(&self) -> usize {
        let before = &self.text[..self.cursor];
        let mut seen_word = false;
        for (i, c) in before.char_indices().rev() {
            if is_word(c) {
                seen_word = true;
            } else if seen_word {
                return i + c.len_utf8();
            }
        }
        0
    }

    fn word_end_after(&self) -> usize {
        let mut seen_word = false;
        for (i, c) in self.text[self.cursor..].char_indices() {
            if is_word(c) {
                seen_word = true;
            } else if seen_word {
                return self.cursor + i;
            }
        }
        self.text.len()
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end_after();
    }

    /// Delete the word before the cursor (Ctrl-W, Alt-Backspace).
    pub fn delete_word_back(&mut self) {
        let start = self.word_start_before();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Delete to the end of the line (Ctrl-K); at the end of a line, join the next one.
    pub fn kill_to_end(&mut self) {
        let end = self.line_end();
        if end == self.cursor {
            self.delete();
        } else {
            self.text.replace_range(self.cursor..end, "");
        }
    }

    /// Delete to the start of the line (Ctrl-U).
    pub fn kill_to_start(&mut self) {
        let start = self.line_start();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// The line the cursor is on and its column in characters.
    fn line_and_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.matches('\n').count();
        let col = before[self.line_start()..].chars().count();
        (line, col)
    }

    /// Put the cursor on line `line` at column `col` (in characters), or the line's end.
    fn go_to(&mut self, line: usize, col: usize) {
        let mut start = 0;
        for (i, l) in self.text.split('\n').enumerate() {
            if i == line {
                let off = l.char_indices().nth(col).map_or(l.len(), |(o, _)| o);
                self.cursor = start + off;
                return;
            }
            start += l.len() + 1;
        }
    }

    /// One line up; false when already on the first line (the caller goes back in history).
    pub fn up(&mut self) -> bool {
        let (line, col) = self.line_and_col();
        if line == 0 {
            return false;
        }
        self.go_to(line - 1, col);
        true
    }

    /// One line down; false when already on the last line.
    pub fn down(&mut self) -> bool {
        let (line, col) = self.line_and_col();
        if line + 1 >= self.text.split('\n').count() {
            return false;
        }
        self.go_to(line + 1, col);
        true
    }

    /// The text wrapped to rows of at most `width` columns (wide characters count 2), and the
    /// row and column of the cursor in them. A row never ends in the middle of a character.
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows: Vec<String> = vec![String::new()];
        let mut cursor = (0, 0);
        let mut col = 0;
        for (i, c) in self.text.char_indices() {
            if i == self.cursor {
                cursor = (rows.len() - 1, col);
            }
            if c == '\n' {
                rows.push(String::new());
                col = 0;
                continue;
            }
            let w = char_width(c);
            if col + w > width {
                rows.push(String::new());
                col = 0;
                if i == self.cursor {
                    cursor = (rows.len() - 1, 0);
                }
            }
            if let Some(row) = rows.last_mut() {
                row.push(c);
            }
            col += w;
        }
        if self.cursor >= self.text.len() {
            // The cursor after the last character: on a new row when that one is full.
            let last = rows.last().map_or(0, |r| display_width(r));
            if last >= width {
                rows.push(String::new());
                cursor = (rows.len() - 1, 0);
            } else {
                cursor = (rows.len() - 1, last);
            }
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str, cursor: usize) -> Editor {
        Editor { text: text.into(), cursor }
    }

    #[test]
    fn typing_and_deleting_keep_the_cursor_on_character_boundaries() {
        let mut e = Editor::default();
        for c in "héllo 日本".chars() {
            e.insert_char(c);
        }
        assert_eq!(e.text(), "héllo 日本");
        e.backspace();
        e.left();
        e.left();
        e.delete();
        assert_eq!(e.text(), "héllo日");
        e.home();
        e.right();
        e.backspace();
        assert_eq!((e.text(), e.cursor()), ("éllo日", 0));
    }

    #[test]
    fn word_moves_and_kills() {
        let mut e = ed("cargo test --quiet", 18);
        e.word_left();
        assert_eq!(e.cursor(), 13);
        e.delete_word_back();
        assert_eq!(e.text(), "cargo quiet");
        e.word_right();
        assert_eq!(e.cursor(), 11);
        e.home();
        e.word_right();
        assert_eq!(e.cursor(), 5);
        e.kill_to_end();
        assert_eq!(e.text(), "cargo");
        e.kill_to_start();
        assert_eq!((e.text(), e.cursor()), ("", 0));
    }

    #[test]
    fn up_and_down_move_between_lines_and_say_when_they_cannot() {
        let mut e = ed("first line\nsecond", 17);
        assert!(!e.down());
        assert!(e.up());
        assert_eq!(e.cursor(), 6);
        assert!(!e.up());
        assert!(e.down());
        assert_eq!(e.cursor(), 17);
    }

    #[test]
    fn paste_keeps_line_breaks_and_drops_control_characters() {
        let mut e = Editor::default();
        e.insert_str("a\r\nb\tc\x07\rd");
        assert_eq!(e.text(), "a\nb    c\nd");
    }

    #[test]
    fn layout_wraps_by_display_width_and_places_the_cursor() {
        let e = ed("abcdef", 6);
        assert_eq!(e.layout(4), (vec!["abcd".into(), "ef".into()], (1, 2)));
        let full = ed("abcd", 4);
        assert_eq!(full.layout(4), (vec!["abcd".into(), String::new()], (1, 0)));
        let wide = ed("日本語", 6);
        assert_eq!(wide.layout(5), (vec!["日本".into(), "語".into()], (1, 0)));
        let lines = ed("a\nbc", 1);
        assert_eq!(lines.layout(10), (vec!["a".into(), "bc".into()], (0, 1)));
        assert_eq!(Editor::default().layout(10), (vec![String::new()], (0, 0)));
    }

    #[test]
    fn completion_replaces_the_word_before_the_cursor() {
        let mut e = ed("/mo", 3);
        e.replace_before_cursor(0, "/model ");
        assert_eq!((e.text(), e.cursor()), ("/model ", 7));
    }
}
