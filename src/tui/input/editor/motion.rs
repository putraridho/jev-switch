//! Cursor motion: characters, words, line ends, and up and down between lines.

use super::{is_word, Editor};

impl Editor {
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

    pub(super) fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    pub(super) fn line_end(&self) -> usize {
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

    pub(super) fn word_start_before(&self) -> usize {
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

}
