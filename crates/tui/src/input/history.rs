//! Earlier messages, for ↑/↓ and Ctrl-R. Kept in `<state dir>/history` (owner-only), in the
//! same `#V2` format the line editor used before, so nothing typed earlier is lost.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::input::line_editor::history::{prepare_history_file, HISTORY_MAX};

const V2: &str = "#V2";

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<String>,
    /// While browsing: the entry shown (an index into `entries`).
    pos: Option<usize>,
    /// What was in the box before browsing started, put back past the newest entry.
    draft: String,
    path: Option<PathBuf>,
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The entries of a history file's text, oldest first.
fn parse(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    let v2 = text.starts_with(V2);
    if v2 {
        lines.next();
    }
    lines.filter(|l| !l.is_empty()).map(|l| if v2 { unescape(l) } else { l.to_string() }).collect()
}

impl History {
    /// Load from `path` (created owner-only when missing); `None` keeps history in memory.
    pub fn load(path: Option<&Path>) -> Self {
        let entries = path
            .and_then(|p| {
                prepare_history_file(p);
                std::fs::read_to_string(p).ok()
            })
            .map(|t| parse(&t))
            .unwrap_or_default();
        let skip = entries.len().saturating_sub(HISTORY_MAX);
        Self { entries: entries.into_iter().skip(skip).collect(), pos: None, draft: String::new(), path: path.map(Path::to_path_buf) }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn from_entries(entries: &[&str]) -> Self {
        Self { entries: entries.iter().map(|s| s.to_string()).collect(), ..Self::default() }
    }

    /// The messages, newest first (for ghost text).
    pub fn newest_first(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().rev()
    }

    /// Whether ↑/↓ browsing is under way (ghost text stays out of the way then).
    pub fn browsing(&self) -> bool {
        self.pos.is_some()
    }

    /// Keep a sent message (not blank, not the same as the last one) and stop browsing.
    pub fn remember(&mut self, line: &str) {
        self.pos = None;
        if line.trim().is_empty() || self.entries.last().is_some_and(|l| l == line) {
            return;
        }
        self.entries.push(line.to_string());
        if let Some(path) = &self.path {
            let fresh = std::fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
                let header = if fresh { format!("{V2}\n") } else { String::new() };
                let _ = f.write_all(format!("{header}{}\n", escape(line)).as_bytes());
            }
        }
    }

    /// The entry before the one shown (↑); `current` is what the box holds now, kept to come
    /// back to. `None` at the oldest entry.
    pub fn older(&mut self, current: &str) -> Option<String> {
        let next = match self.pos {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.pos = Some(next);
        self.entries.get(next).cloned()
    }

    /// The entry after the one shown (↓); past the newest, what the box held before. `None`
    /// when not browsing.
    pub fn newer(&mut self) -> Option<String> {
        let i = self.pos?;
        if i + 1 >= self.entries.len() {
            self.pos = None;
            return Some(std::mem::take(&mut self.draft));
        }
        self.pos = Some(i + 1);
        self.entries.get(i + 1).cloned()
    }

    /// Ctrl-R: the next older entry that contains `needle` (case-insensitive), starting before
    /// the one shown.
    pub fn search(&mut self, needle: &str, current: &str) -> Option<String> {
        let needle = needle.to_lowercase();
        let start = match self.pos {
            None => {
                self.draft = current.to_string();
                self.entries.len()
            }
            Some(i) => i,
        };
        let found = (0..start).rev().find(|&i| self.entries[i].to_lowercase().contains(&needle))?;
        self.pos = Some(found);
        self.entries.get(found).cloned()
    }

    /// Stop browsing (the text was edited).
    pub fn reset(&mut self) {
        self.pos = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_goes_back_and_returns_to_the_draft() {
        let mut h = History::from_entries(&["one", "two"]);
        assert_eq!(h.older("draft").as_deref(), Some("two"));
        assert_eq!(h.older("two").as_deref(), Some("one"));
        assert_eq!(h.older("one"), None);
        assert_eq!(h.newer().as_deref(), Some("two"));
        assert_eq!(h.newer().as_deref(), Some("draft"));
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn search_finds_older_matches_one_by_one() {
        let mut h = History::from_entries(&["cargo build", "git status", "cargo test"]);
        assert_eq!(h.search("CARGO", "").as_deref(), Some("cargo test"));
        assert_eq!(h.search("cargo", "").as_deref(), Some("cargo build"));
        assert_eq!(h.search("cargo", ""), None);
    }

    #[test]
    fn the_file_round_trips_multi_line_entries_in_the_v2_format() {
        let dir = std::env::temp_dir().join(format!("jev-hist-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history");
        let _ = std::fs::remove_file(&path);
        let mut h = History::load(Some(&path));
        h.remember("line one\nline \\two");
        h.remember("line one\nline \\two");
        h.remember("  ");
        h.remember("next");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(text, "#V2\nline one\\nline \\\\two\nnext\n");
        let again = History::load(Some(&path));
        assert_eq!(again.entries, vec!["line one\nline \\two".to_string(), "next".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
