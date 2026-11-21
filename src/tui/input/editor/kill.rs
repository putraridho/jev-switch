//! Deleting text before, after and around the cursor.

use super::Editor;

impl Editor {
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
            // The text now ends where the removed line's break was; `line_start` needs a cursor
            // inside the text to look back from.
            self.cursor = (start - 1).min(self.text.len());
            self.cursor = self.line_start();
        } else {
            self.text.clear();
            self.cursor = 0;
        }
    }
}
