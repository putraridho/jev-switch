//! Automatic session names and the "where we left off" line shown on resume.

use crate::render::{human_duration, truncate_to_width};

pub const MAX_TITLE_COLUMNS: usize = 40;

/// Polite openers that say nothing about the task.
const FILLER: [&str; 8] = ["please ", "can you ", "could you ", "would you ", "i want you to ", "i need you to ", "i want to ", "let's "];

/// A short name from the first message: its first sentence without polite openers, the
/// first letter capitalised, cut at [`MAX_TITLE_COLUMNS`] columns. Empty for a message with
/// no words (a slash command is not a title).
pub fn auto_title(first_message: &str) -> String {
    let text = first_message.trim();
    if text.starts_with('/') {
        return String::new();
    }
    let sentence = text.split(['\n', '.', '?', '!']).map(str::trim).find(|s| !s.is_empty()).unwrap_or("");
    let mut s = sentence.to_string();
    loop {
        let lower = s.to_lowercase();
        match FILLER.iter().find(|f| lower.starts_with(*f)) {
            Some(f) if s.len() > f.len() => s = s[f.len()..].trim_start().to_string(),
            _ => break,
        }
    }
    let mut cs = s.chars();
    let titled = cs.next().map(|c| c.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default();
    truncate_to_width(&titled, MAX_TITLE_COLUMNS)
}

/// What a resumed session shows first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapFacts {
    pub title: String,
    pub last_message: String,
    pub turns: usize,
    pub files_changed: usize,
    pub idle_ms: u64,
}

/// `Resuming "Parser fix": 12 turns, 3 files changed, 2h ago. Last: fix the lexer test`.
pub fn recap_line(f: &RecapFacts) -> String {
    let title = if f.title.is_empty() { "an untitled session".to_string() } else { format!("\"{}\"", f.title) };
    let mut parts = vec![format!("{} turn{}", f.turns, if f.turns == 1 { "" } else { "s" })];
    if f.files_changed > 0 {
        parts.push(format!("{} file{} changed", f.files_changed, if f.files_changed == 1 { "" } else { "s" }));
    }
    parts.push(format!("{} ago", human_duration(f.idle_ms)));
    let last = f.last_message.split_whitespace().collect::<Vec<_>>().join(" ");
    let last = if last.is_empty() { String::new() } else { format!(". Last: {}", truncate_to_width(&last, 60)) };
    format!("Resuming {title}: {}{last}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_first_messages() {
        for (msg, want) in [
            ("fix the parser bug", "Fix the parser bug"),
            ("Please can you fix the parser bug. Then the lexer.", "Fix the parser bug"),
            ("can you   explain this?", "Explain this"),
            ("please", "Please"),
            ("/model codex", ""),
            ("", ""),
            ("\n\nrefactor the router\nand more", "Refactor the router"),
            ("add a really long description of everything that the new feature should do", "Add a really long description of everyt…"),
        ] {
            assert_eq!(auto_title(msg), want, "{msg:?}");
        }
    }

    #[test]
    fn recap_forms() {
        let f = RecapFacts { title: "Parser fix".into(), last_message: "fix the\nlexer test".into(), turns: 12, files_changed: 3, idle_ms: 7_200_000 };
        assert_eq!(recap_line(&f), "Resuming \"Parser fix\": 12 turns, 3 files changed, 2h00m ago. Last: fix the lexer test");
        let bare = RecapFacts { title: String::new(), last_message: String::new(), turns: 1, files_changed: 0, idle_ms: 5_000 };
        assert_eq!(recap_line(&bare), "Resuming an untitled session: 1 turn, 5s ago");
    }
}
