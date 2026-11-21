//! The session library: every saved session of every project, searched, listed with its
//! marks (tags, pinned, archived), exported to a file and imported back, measured and
//! cleaned. Pure over a state folder, so each rule is a unit test on a temp folder; the
//! `/sessions` command in the REPL only maps text onto these functions.

pub mod marks;
pub mod query;
pub mod search;
pub mod transfer;
pub mod upkeep;

use std::path::{Path, PathBuf};

use crate::backend::ProviderId;
use crate::session::domain::saved_session::SavedSession;
use crate::session::application::ports::SessionStore;
use crate::session::domain::session_meta::Meta;

/// One day in milliseconds, the unit of the library's age filters.
pub const DAY_MS: u64 = 86_400_000;

/// One saved session found on disk, with what is said about it.
#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub saved: SavedSession,
    pub bytes: u64,
    pub meta: Meta,
}

impl LibraryEntry {
    pub fn title(&self) -> String {
        self.saved.sessions.title().unwrap_or_else(|| "(no message)".to_string())
    }

    /// Everything searchable about the session, lower-cased, one line per piece.
    fn haystack(&self) -> Vec<String> {
        let mut lines = vec![self.title().to_lowercase()];
        for turn in self.saved.sessions.log() {
            lines.extend(turn.prompt.lines().map(str::to_lowercase));
            lines.extend(turn.reply.lines().map(str::to_lowercase));
            lines.extend(turn.tools.iter().map(|t| t.to_lowercase()));
            lines.push(format!("{} {}", turn.provider, turn.model).to_lowercase());
        }
        lines
    }

    fn mentions_file(&self, needle: &str) -> bool {
        self.saved.sessions.log().iter().any(|t| t.tools.iter().any(|tool| tool.to_lowercase().contains(needle)))
    }

    fn used(&self, provider: ProviderId) -> bool {
        self.saved.sessions.log().iter().any(|t| t.provider == provider)
    }
}

fn session_dirs(state_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(state_dir.join("sessions")).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
}

/// Every readable session file under `<state>/sessions/`, in any project, newest first.
/// Damaged files are skipped (they stay on disk; `clean` removes them).
pub fn scan_all(store: &dyn SessionStore, state_dir: &Path) -> Vec<LibraryEntry> {
    let meta = store.load_marks(state_dir);
    let files = session_dirs(state_dir).into_iter().flat_map(|p| {
        if p.is_dir() {
            std::fs::read_dir(p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
        } else {
            vec![p]
        }
    });
    let mut found: Vec<LibraryEntry> = files
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let saved = store.parse_session(&text).ok().filter(|s| s.sessions.turns() > 0)?;
            Some(LibraryEntry { bytes: text.len() as u64, meta: meta.get(&path), path, saved })
        })
        .collect();
    found.sort_by(|a, b| (b.saved.saved_ms, &b.path).cmp(&(a.saved.saved_ms, &a.path)));
    found
}


#[cfg(test)]
mod tests;
