//! The text in the input box and the cursor in it: plain data and pure edits, so every key's
//! effect is a unit test. Lines are separated by `\n`; the cursor is a byte offset that always
//! sits on a character boundary.

#[cfg(test)]
mod tests;
mod kill;
mod layout;
mod motion;

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
}
