//! The conversation, kept by the app in the full-screen layout (inline, the terminal's own
//! scrollback keeps it). Lines are already styled and laid out for the chat pane; the pane
//! shows the newest ones, or an older stretch while the person scrolls back.

/// The most lines kept; the oldest go first.
const MAX_LINES: usize = 20_000;

#[derive(Debug, Default)]
pub struct Transcript {
    lines: Vec<String>,
    /// How many lines the view is above the newest (0 follows the end).
    scroll: usize,
}

impl Transcript {
    /// Add finished output. While scrolled back the view stays on what is being read.
    pub fn push_text(&mut self, text: &str) {
        let before = self.lines.len();
        self.lines.extend(text.split('\n').map(str::to_string));
        let added = self.lines.len() - before;
        if self.scroll > 0 {
            self.scroll += added;
        }
        if self.lines.len() > MAX_LINES {
            let drop = self.lines.len() - MAX_LINES;
            self.lines.drain(..drop);
            self.scroll = self.scroll.saturating_sub(drop);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll = 0;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Lines the view is above the end.
    pub fn scrolled(&self) -> usize {
        self.scroll
    }

    /// Move the view `by` lines (positive goes back in time) in a pane `height` rows tall.
    pub fn scroll_by(&mut self, by: isize, height: usize) {
        let max = self.lines.len().saturating_sub(height);
        let next = if by >= 0 { self.scroll.saturating_add(by as usize) } else { self.scroll.saturating_sub(by.unsigned_abs()) };
        self.scroll = next.min(max);
    }

    pub fn to_end(&mut self) {
        self.scroll = 0;
    }

    pub fn to_start(&mut self, height: usize) {
        self.scroll = self.lines.len().saturating_sub(height);
    }

    /// Exactly `height` rows: the newest lines (or the stretch scrolled to), the rest blank
    /// above when the conversation is shorter than the pane.
    pub fn window(&self, height: usize) -> Vec<String> {
        let end = self.lines.len().saturating_sub(self.scroll.min(self.lines.len().saturating_sub(height)));
        let start = end.saturating_sub(height);
        let mut rows: Vec<String> = Vec::with_capacity(height);
        rows.resize(height.saturating_sub(end - start), String::new());
        rows.extend(self.lines[start..end].iter().cloned());
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Transcript {
        let mut t = Transcript::default();
        t.push_text(&(1..=n).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"));
        t
    }

    #[test]
    fn a_short_conversation_sits_at_the_bottom_of_the_pane() {
        assert_eq!(lines(2).window(4), vec!["", "", "l1", "l2"]);
    }

    #[test]
    fn the_pane_follows_the_end_and_scrolls_back_and_clamps() {
        let mut t = lines(10);
        assert_eq!(t.window(3), vec!["l8", "l9", "l10"]);
        t.scroll_by(4, 3);
        assert_eq!(t.window(3), vec!["l4", "l5", "l6"]);
        t.scroll_by(100, 3);
        assert_eq!(t.window(3), vec!["l1", "l2", "l3"], "stops at the first line");
        t.scroll_by(-100, 3);
        assert_eq!(t.window(3), vec!["l8", "l9", "l10"]);
    }

    #[test]
    fn new_output_does_not_move_a_view_scrolled_back() {
        let mut t = lines(10);
        t.scroll_by(2, 3);
        assert_eq!(t.window(3), vec!["l6", "l7", "l8"]);
        t.push_text("a\nb");
        assert_eq!(t.window(3), vec!["l6", "l7", "l8"]);
        t.to_end();
        assert_eq!(t.window(3), vec!["l10", "a", "b"]);
    }

    #[test]
    fn the_oldest_lines_go_first_when_it_gets_long() {
        let mut t = Transcript::default();
        t.push_text(&"x\n".repeat(MAX_LINES + 50));
        assert_eq!(t.len(), MAX_LINES);
    }
}
