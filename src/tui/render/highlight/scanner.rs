//! The line scanner: turns one line into runs of plain text, keywords, strings, comments and numbers.

use super::spec::Spec;
use super::{Kind, LineState};

/// Where a scan is: the characters of the line and what has been found so far.
struct Scanner<'a> {
    chars: Vec<char>,
    spec: &'a Spec,
    out: Vec<(Kind, String)>,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// End of a block comment that continues from `from`, and whether it closed on this line.
fn block_comment_end(chars: &[char], from: usize) -> (usize, bool) {
    match (from..chars.len().saturating_sub(1)).find(|&j| chars[j] == '*' && chars[j + 1] == '/') {
        Some(j) => (j + 2, true),
        None => (chars.len(), false),
    }
}

/// End of the string that opens with `quote` at `start`; a backslash escapes the next
/// character, and an unclosed string runs to the end of the line.
fn string_end(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    chars.len()
}

/// End of the number that starts at `start` (digits, letters, `_`, and `.` before a digit).
fn number_end(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && (is_ident_char(chars[i]) || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
        i += 1;
    }
    i
}

fn ident_end(chars: &[char], start: usize) -> usize {
    start + chars[start..].iter().take_while(|c| is_ident_char(**c)).count()
}

impl Scanner<'_> {
    fn push(&mut self, kind: Kind, from: usize, to: usize) {
        let text: String = self.chars[from..to].iter().collect();
        match self.out.last_mut() {
            Some((k, t)) if *k == kind => t.push_str(&text),
            _ => self.out.push((kind, text)),
        }
    }

    fn starts_with(&self, at: usize, pat: &str) -> bool {
        pat.chars().enumerate().all(|(k, p)| self.chars.get(at + k) == Some(&p))
    }

    /// Whether a line comment starts at `at`. A `#` only counts at the start of a word,
    /// so `$#` and `a#b` are not comments.
    fn line_comment_at(&self, at: usize) -> bool {
        self.spec.line_comments.iter().any(|p| {
            self.starts_with(at, p) && (*p != "#" || at == 0 || !(is_ident_char(self.chars[at - 1]) || self.chars[at - 1] == '$'))
        })
    }

    fn is_keyword(&self, word: &str) -> bool {
        if self.spec.ignore_case {
            self.spec.keywords.contains(&word.to_ascii_lowercase().as_str())
        } else {
            self.spec.keywords.contains(&word)
        }
    }

    /// Scan one token at `i` and return where the next one starts.
    fn step(&mut self, i: usize, state: &mut LineState) -> usize {
        let c = self.chars[i];
        if self.line_comment_at(i) {
            self.push(Kind::Comment, i, self.chars.len());
            return self.chars.len();
        }
        if self.spec.block_comments && self.starts_with(i, "/*") {
            return self.block_comment(i + 2, i, state);
        }
        if self.spec.quotes.contains(&c) {
            let end = string_end(&self.chars, i, c);
            self.push(Kind::Str, i, end);
            return end;
        }
        if c.is_ascii_digit() {
            let end = number_end(&self.chars, i);
            self.push(Kind::Number, i, end);
            return end;
        }
        if is_ident_char(c) {
            let end = ident_end(&self.chars, i);
            let word: String = self.chars[i..end].iter().collect();
            self.push(if self.is_keyword(&word) { Kind::Keyword } else { Kind::Plain }, i, end);
            return end;
        }
        self.push(Kind::Plain, i, i + 1);
        i + 1
    }

    /// Colour a block comment that began at `from`, searching for its end after `search`.
    fn block_comment(&mut self, search: usize, from: usize, state: &mut LineState) -> usize {
        let (end, closed) = block_comment_end(&self.chars, search);
        self.push(Kind::Comment, from, end);
        *state = if closed { LineState::Normal } else { LineState::InBlockComment };
        end
    }

    fn run(&mut self, state: &mut LineState) {
        let mut i = 0;
        if *state == LineState::InBlockComment {
            i = self.block_comment(0, 0, state);
        }
        while i < self.chars.len() {
            i = self.step(i, state);
        }
    }
}

/// `(kind, text)` runs of `line`, and the state to carry to the next line.
pub(super) fn tokenize(spec: &Spec, line: &str, state: LineState) -> (Vec<(Kind, String)>, LineState) {
    let mut scanner = Scanner { chars: line.chars().collect(), spec, out: Vec::new() };
    let mut next = state;
    scanner.run(&mut next);
    (scanner.out, next)
}
