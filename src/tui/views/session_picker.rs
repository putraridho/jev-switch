//! The `/resume` list, like Claude Code's: the saved sessions of the folder, newest first,
//! each with what was first asked and when it was last used. Arrow keys move, Enter takes
//! the one under the cursor, Esc leaves, and typing narrows the list.
//!
//! The state, the key decoding and the drawing are plain functions of their input so they
//! are tested; only [`run`] touches the terminal.

use crate::session::domain::saved_session::SessionEntry;
use crate::tui::render::{display_width, truncate_to_width, Style};
use crate::tui::views::status_facts::human_ago;

/// A key the list understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    /// Esc alone, or Ctrl-D.
    Cancel,
    /// Ctrl-C.
    Interrupt,
    Backspace,
    Char(char),
}

/// What a key did to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// The session at this index of the entries was chosen.
    Chosen(usize),
    Cancelled,
}

/// The keys in one read from the terminal: an escape sequence arrives whole, typed or
/// pasted text arrives as many characters.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    match bytes {
        [] => vec![],
        [0x1b] => vec![Key::Cancel],
        [0x1b, b'[' | b'O', b'A'] => vec![Key::Up],
        [0x1b, b'[' | b'O', b'B'] => vec![Key::Down],
        [0x1b, b'[', b'5', b'~'] => vec![Key::PageUp],
        [0x1b, b'[', b'6', b'~'] => vec![Key::PageDown],
        // Any other escape sequence (Left, Right, Home, End, ...) says nothing.
        [0x1b, ..] => vec![],
        _ => String::from_utf8_lossy(bytes).chars().filter_map(key_for_char).collect(),
    }
}

fn key_for_char(c: char) -> Option<Key> {
    match c {
        '\r' | '\n' => Some(Key::Enter),
        '\u{7f}' | '\u{8}' => Some(Key::Backspace),
        '\u{3}' => Some(Key::Interrupt),
        '\u{4}' => Some(Key::Cancel),
        '\u{10}' => Some(Key::Up),
        '\u{e}' => Some(Key::Down),
        c if c.is_control() => None,
        c => Some(Key::Char(c)),
    }
}

/// The list and where the cursor is.
pub struct Picker {
    entries: Vec<SessionEntry>,
    filter: String,
    /// The cursor, as a position among the entries the filter lets through.
    selected: usize,
    /// The first row of the page shown.
    offset: usize,
    page: usize,
}

impl Picker {
    /// `page`: how many sessions are shown at once.
    pub fn new(entries: Vec<SessionEntry>, page: usize) -> Self {
        Self { entries, filter: String::new(), selected: 0, offset: 0, page: page.max(1) }
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Indexes into the entries of those that match what was typed, in list order.
    pub fn visible(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|&i| matches(&self.entries[i], &self.filter)).collect()
    }

    pub fn press(&mut self, key: Key) -> Outcome {
        let shown = self.visible();
        let last = shown.len().saturating_sub(1);
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(last),
            Key::PageUp => self.selected = self.selected.saturating_sub(self.page),
            Key::PageDown => self.selected = (self.selected + self.page).min(last),
            Key::Enter => {
                if let Some(&i) = shown.get(self.selected) {
                    return Outcome::Chosen(i);
                }
            }
            Key::Cancel | Key::Interrupt => return Outcome::Cancelled,
            Key::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            Key::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
        }
        // Keep the cursor on the page.
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.page {
            self.offset = self.selected + 1 - self.page;
        }
        if self.offset > self.selected || self.visible().len() <= self.page {
            self.offset = if self.visible().len() <= self.page { 0 } else { self.offset.min(self.selected) };
        }
        Outcome::Continue
    }

    /// The lines to draw: title, search, the page of sessions (two lines each), footer.
    pub fn render(&self, now_ms: u64, width: usize, style: Style) -> Vec<String> {
        let width = width.max(20);
        let shown = self.visible();
        let mut out = vec![style.bold("Resume a session"), String::new()];
        let search = if self.filter.is_empty() { style.dim("type to search") } else { self.filter.clone() };
        out.push(format!("  {} {search}", style.dim("Search:")));
        out.push(String::new());
        if shown.is_empty() {
            out.push(format!("  {}", style.dim("No session matches.")));
        }
        for (pos, &i) in shown.iter().enumerate().skip(self.offset).take(self.page) {
            let e = &self.entries[i];
            let title = truncate_to_width(&clean(&e.title), width.saturating_sub(4));
            let turns = if e.info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", e.info.turns) };
            let last = e.info.current.as_ref().map_or(String::new(), |(p, m)| format!(" · {p} {m}"));
            let detail = truncate_to_width(&format!("{} · {turns}{last}", human_ago(now_ms.saturating_sub(e.info.last_used_ms))), width.saturating_sub(4));
            if pos == self.selected {
                out.push(format!("{} {}", style.cyan("❯"), style.bold(&title)));
                out.push(format!("  {}", style.dim(&detail)));
            } else {
                out.push(format!("  {title}"));
                out.push(format!("  {}", style.dim(&detail)));
            }
        }
        if shown.len() > self.page {
            let end = (self.offset + self.page).min(shown.len());
            out.push(style.dim(&format!("  {}-{} of {}", self.offset + 1, end, shown.len())));
        }
        out.push(String::new());
        out.push(style.dim(&truncate_to_width("  ↑↓ select · Enter resume · Esc cancel · type to search", width)));
        out
    }
}

/// A line of text with nothing in it that could move the cursor.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Does the session match what was typed: every word of it, in any case, is somewhere in
/// the first message or the providers and models it ran on.
pub fn matches(e: &SessionEntry, filter: &str) -> bool {
    let mut hay = e.title.to_lowercase();
    for p in &e.info.providers {
        hay.push(' ');
        hay.push_str(&p.to_string().to_lowercase());
    }
    if let Some((p, m)) = &e.info.current {
        hay.push_str(&format!(" {p} {m}").to_lowercase());
    }
    filter.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// How many sessions fit at once on a screen of `rows` rows (two lines each, and seven for
/// the title, search, counter and footer), at most ten.
pub fn page_for(rows: Option<usize>) -> usize {
    rows.map_or(8, |r| (r.saturating_sub(9) / 2).clamp(1, 10))
}

/// Show the list and wait for a choice: the index of the chosen entry, or `None` when the
/// person left. Without a terminal to read keys from there is nobody to choose: `None`.
#[cfg(unix)]
pub fn run(entries: Vec<SessionEntry>, now_ms: u64, style: Style, rows: Option<usize>, width: usize) -> Option<usize> {
    use super::raw_keys::{Block, RawKeys, Read};

    let keys = RawKeys::enter()?;
    let mut picker = Picker::new(entries, page_for(rows));
    let mut block = Block::default();
    block.draw(&picker.render(now_ms, width, style));
    let chosen = 'keys: loop {
        let bytes = match keys.read(None) {
            Read::Bytes(bytes) => bytes,
            Read::Timeout => continue,
            Read::Closed => break None,
        };
        for key in decode(&bytes) {
            match picker.press(key) {
                Outcome::Continue => {}
                Outcome::Chosen(i) => break 'keys Some(i),
                Outcome::Cancelled => break 'keys None,
            }
        }
        block.draw(&picker.render(now_ms, width, style));
    };
    block.clear();
    chosen
}

#[cfg(not(unix))]
pub fn run(_entries: Vec<SessionEntry>, _now_ms: u64, _style: Style, _rows: Option<usize>, _width: usize) -> Option<usize> {
    None
}

/// The list as plain lines, numbered, for a terminal that cannot take keys: `/resume 2`
/// then picks one.
pub fn numbered(entries: &[SessionEntry], now_ms: u64, width: usize, style: Style) -> Vec<String> {
    entries
        .iter()
        .enumerate()
        .flat_map(|(n, e)| {
            let turns = if e.info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", e.info.turns) };
            let head = format!("{:>3}. ", n + 1);
            let room = width.saturating_sub(display_width(&head));
            [
                format!("{head}{}", truncate_to_width(&clean(&e.title), room)),
                format!("     {}", style.dim(&format!("{} · {turns}", human_ago(now_ms.saturating_sub(e.info.last_used_ms))))),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ProviderId;
    use crate::session::domain::saved_session::SessionInfo;
    use std::path::PathBuf;

    const NOW: u64 = 10 * 86_400_000;

    fn entry(title: &str, turns: usize, ago_min: u64) -> SessionEntry {
        SessionEntry {
            path: PathBuf::from(format!("{title}.json")),
            info: SessionInfo {
                turns,
                last_used_ms: NOW - ago_min * 60_000,
                providers: vec![ProviderId::Claude],
                current: Some((ProviderId::Claude, "sonnet".into())),
            },
            title: title.into(),
        }
    }

    fn three() -> Vec<SessionEntry> {
        vec![entry("fix the parser", 4, 5), entry("add a settings page", 1, 120), entry("rename the crate", 9, 3000)]
    }

    #[test]
    fn escape_sequences_and_text_decode_to_keys() {
        assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
        assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
        assert_eq!(decode(b"\x1b"), vec![Key::Cancel]);
        assert_eq!(decode(b"\x1b[C"), vec![]);
        assert_eq!(decode(b"\r"), vec![Key::Enter]);
        assert_eq!(decode(b"\x7f"), vec![Key::Backspace]);
        assert_eq!(decode(b"\x03"), vec![Key::Interrupt]);
        assert_eq!(decode("aé".as_bytes()), vec![Key::Char('a'), Key::Char('é')]);
        assert_eq!(decode(b"\t"), vec![]);
    }

    #[test]
    fn the_cursor_moves_and_stops_at_the_ends() {
        let mut p = Picker::new(three(), 5);
        assert_eq!(p.press(Key::Up), Outcome::Continue);
        assert_eq!(p.press(Key::Down), Outcome::Continue);
        assert_eq!(p.press(Key::Down), Outcome::Continue);
        assert_eq!(p.press(Key::Down), Outcome::Continue);
        assert_eq!(p.press(Key::Enter), Outcome::Chosen(2));
    }

    #[test]
    fn enter_takes_the_first_session_by_default_and_esc_leaves() {
        assert_eq!(Picker::new(three(), 5).press(Key::Enter), Outcome::Chosen(0));
        assert_eq!(Picker::new(three(), 5).press(Key::Cancel), Outcome::Cancelled);
        assert_eq!(Picker::new(three(), 5).press(Key::Interrupt), Outcome::Cancelled);
    }

    #[test]
    fn typing_narrows_the_list_and_enter_takes_the_match() {
        let mut p = Picker::new(three(), 5);
        for c in "SETT".chars() {
            p.press(Key::Char(c));
        }
        assert_eq!(p.visible(), vec![1]);
        assert_eq!(p.press(Key::Enter), Outcome::Chosen(1));
    }

    #[test]
    fn backspace_widens_it_again_and_nothing_matching_takes_nothing() {
        let mut p = Picker::new(three(), 5);
        p.press(Key::Char('z'));
        assert!(p.visible().is_empty());
        assert_eq!(p.press(Key::Enter), Outcome::Continue);
        p.press(Key::Backspace);
        assert_eq!(p.visible().len(), 3);
    }

    #[test]
    fn every_word_must_match_and_the_provider_counts() {
        let e = entry("fix the parser", 4, 5);
        assert!(matches(&e, "parser fix"));
        assert!(!matches(&e, "parser nope"));
        assert!(matches(&e, "claude"));
        assert!(matches(&e, ""));
    }

    #[test]
    fn the_page_follows_the_cursor() {
        let entries: Vec<_> = (0..6).map(|i| entry(&format!("task {i}"), 1, i)).collect();
        let mut p = Picker::new(entries, 2);
        for _ in 0..3 {
            p.press(Key::Down);
        }
        let text = p.render(NOW, 60, Style::off()).join("\n");
        assert!(text.contains("❯ task 3"), "{text}");
        assert!(!text.contains("task 0"), "{text}");
        assert!(text.contains("3-4 of 6"), "{text}");
        p.press(Key::PageUp);
        let text = p.render(NOW, 60, Style::off()).join("\n");
        assert!(text.contains("❯ task 1"), "{text}");
    }

    #[test]
    fn the_list_shows_what_was_asked_and_when() {
        let lines = Picker::new(three(), 5).render(NOW, 70, Style::off());
        let text = lines.join("\n");
        assert!(text.contains("❯ fix the parser"), "{text}");
        assert!(text.contains("5m ago · 4 turns · claude sonnet"), "{text}");
        assert!(text.contains("2h ago · 1 turn · claude sonnet"), "{text}");
        assert!(text.contains("2d ago"), "{text}");
        assert!(text.contains("Esc cancel"), "{text}");
        assert!(lines.iter().all(|l| display_width(l) <= 70), "{text}");
    }

    #[test]
    fn a_title_cannot_move_the_cursor_or_overflow() {
        let long = format!("go\x1b[2J{}", "x".repeat(200));
        let lines = Picker::new(vec![entry(&long, 1, 1)], 5).render(NOW, 40, Style::off());
        assert!(lines.iter().all(|l| !l.contains('\x1b') && display_width(l) <= 40), "{lines:?}");
    }

    #[test]
    fn the_page_fits_the_screen() {
        assert_eq!(page_for(Some(24)), 7);
        assert_eq!(page_for(Some(10)), 1);
        assert_eq!(page_for(Some(200)), 10);
        assert_eq!(page_for(None), 8);
    }

    #[test]
    fn the_plain_list_is_numbered() {
        let lines = numbered(&three(), NOW, 60, Style::off());
        assert_eq!(lines[0], "  1. fix the parser");
        assert_eq!(lines[1], "     5m ago · 4 turns");
        assert_eq!(lines[4], "  3. rename the crate");
    }
}
