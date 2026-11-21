//! Wrapping the text into rows of a given width and finding the cursor in them.

use super::Editor;
use crate::tui::render::{char_width, display_width};

impl Editor {
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
