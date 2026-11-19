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

    /// Left, but never onto the previous line (vim `h`).
    pub fn left_in_line(&mut self) {
        if self.cursor > self.line_start() {
            self.left();
        }
    }

    /// Right, but never past the line's last character (vim `l`).
    pub fn right_in_line(&mut self) {
        let end = self.line_end();
        if self.next_boundary().is_some_and(|j| j < end) {
            self.right();
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

    /// Delete the whole line the cursor is on, with its line break (vim `dd`); the cursor goes
    /// to the start of the line that takes its place.
    pub fn delete_line(&mut self) {
        let start = self.line_start();
        let end = self.line_end();
        if end < self.text.len() {
            self.text.replace_range(start..=end, "");
            self.cursor = start;
        } else if start > 0 {
            self.text.replace_range(start - 1..end, "");
            self.cursor = self.line_start();
        } else {
            self.text.clear();
            self.cursor = 0;
        }
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
    /// row and column of the cursor in them. Rows break after a space, so a word is never cut
    /// unless it alone is wider than a row; a row never ends in the middle of a character.
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows: Vec<String> = Vec::new();
        let mut cursor = (0, 0);
        let mut found = false;
        let mut line_start = 0;
        for line in self.text.split('\n') {
            let segs = wrap_segments(line, width);
            for (i, &(a, b)) in segs.iter().enumerate() {
                let (abs_a, abs_b) = (line_start + a, line_start + b);
                let next = segs.get(i + 1).map(|n| line_start + n.0);
                let inside = match next {
                    Some(n) => self.cursor >= abs_a && self.cursor < n,
                    None => self.cursor >= abs_a && self.cursor <= abs_b,
                };
                if !found && inside {
                    cursor = (rows.len(), display_width(&self.text[abs_a..self.cursor.min(abs_b)]));
                    found = true;
                }
                rows.push(line[a..b].to_string());
            }
            line_start += line.len() + 1;
        }
        if self.cursor >= self.text.len() && rows.last().is_some_and(|r| display_width(r) >= width) {
            // The cursor after the last character: on a new row when that one is full.
            rows.push(String::new());
            cursor = (rows.len() - 1, 0);
        }
        (rows, cursor)
    }
}

/// Where one line (no newline in it) breaks into rows of at most `width` columns: byte ranges
/// of each row's text. The space a row breaks at is left out of both rows.
fn wrap_segments(line: &str, width: usize) -> Vec<(usize, usize)> {
    let mut segs = Vec::new();
    let (mut start, mut col) = (0, 0);
    // The last space in the current row: where its text would end, and where the next starts.
    let mut brk: Option<(usize, usize)> = None;
    for (idx, c) in line.char_indices() {
        let w = char_width(c);
        if c == ' ' {
            if col + w <= width {
                brk = Some((idx, idx + 1));
                col += w;
            } else {
                segs.push((start, idx));
                (start, col, brk) = (idx + 1, 0, None);
            }
            continue;
        }
        while col + w > width {
            match brk.take() {
                Some((end, next)) => {
                    segs.push((start, end));
                    start = next;
                    col = display_width(&line[next..idx]);
                }
                None => {
                    segs.push((start, idx));
                    start = idx;
                    col = 0;
                }
            }
        }
        col += w;
    }
    segs.push((start, line.len()));
    segs
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
    fn layout_breaks_at_spaces_and_never_inside_a_word_that_fits() {
        let e = ed("the quick brown fox", 19);
        assert_eq!(e.layout(10), (vec!["the quick".into(), "brown fox".into()], (1, 9)));
        // The cursor on the space a row breaks at sits at the end of that row.
        let e = ed("the quick brown fox", 9);
        assert_eq!(e.layout(10), (vec!["the quick".into(), "brown fox".into()], (0, 9)));
        let e = ed("the quick brown fox", 10);
        assert_eq!(e.layout(10).1, (1, 0));
        // A word wider than a row is still cut by column; a row's text is never lost.
        let e = ed("a supercalifragilistic b", 24);
        let (rows, _) = e.layout(8);
        assert_eq!(rows.join(" ").replace("  ", " "), "a supercal ifragili stic b".replace("  ", " "));
        assert!(rows.iter().all(|r| display_width(r) <= 8), "{rows:?}");
    }

    #[test]
    fn completion_replaces_the_word_before_the_cursor() {
        let mut e = ed("/mo", 3);
        e.replace_before_cursor(0, "/model ");
        assert_eq!((e.text(), e.cursor()), ("/model ", 7));
    }
}
