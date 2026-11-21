//! What the person says about a saved session: tags, pinned, archived. Kept beside the
//! sessions, not inside them, so a session file stays exactly what the engine wrote.
//! Entries are keyed by the session file's name without the extension. The file they are
//! read from and written to is `session::infrastructure::session_meta`.

use std::collections::BTreeMap;
use std::path::Path;

/// One session's marks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub tags: Vec<String>,
    /// Kept at the top of lists and never deleted by the keep-ten rule or `/sessions clean`.
    pub pinned: bool,
    /// Left out of `/resume` and of the default listing; searchable with `archived`.
    pub archived: bool,
}

impl Meta {
    fn is_blank(&self) -> bool {
        self.tags.is_empty() && !self.pinned && !self.archived
    }
}

/// A tag as stored: one lowercase word of letters, digits, `-`, `_`, `.`.
pub fn clean_tag(raw: &str) -> Option<String> {
    let tag = raw.trim().trim_start_matches('#').to_lowercase();
    let ok = !tag.is_empty() && tag.len() <= 32 && tag.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'));
    ok.then_some(tag)
}

/// The key a session file is known by.
pub fn key_of(session_file: &Path) -> String {
    session_file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The marks of every session of one state folder, in memory.
#[derive(Debug, Clone, Default)]
pub struct MetaStore {
    entries: BTreeMap<String, Meta>,
}

impl MetaStore {
    pub fn from_entries(entries: BTreeMap<String, Meta>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &BTreeMap<String, Meta> {
        &self.entries
    }

    pub fn get(&self, session_file: &Path) -> Meta {
        self.entries.get(&key_of(session_file)).cloned().unwrap_or_default()
    }

    pub fn is_pinned(&self, session_file: &Path) -> bool {
        self.get(session_file).pinned
    }

    pub fn is_archived(&self, session_file: &Path) -> bool {
        self.get(session_file).archived
    }

    /// Change the marks of one session; a session with no marks left is forgotten.
    pub fn update(&mut self, session_file: &Path, change: impl FnOnce(&mut Meta)) {
        let key = key_of(session_file);
        let mut meta = self.entries.remove(&key).unwrap_or_default();
        change(&mut meta);
        meta.tags.sort();
        meta.tags.dedup();
        if !meta.is_blank() {
            self.entries.insert(key, meta);
        }
    }

    /// Drop the marks of a deleted session.
    pub fn forget(&mut self, session_file: &Path) {
        self.entries.remove(&key_of(session_file));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_one_clean_word() {
        assert_eq!(clean_tag("#Bug-Fix"), Some("bug-fix".into()));
        assert_eq!(clean_tag("two words"), None);
        assert_eq!(clean_tag("  "), None);
        assert_eq!(clean_tag(&"x".repeat(40)), None);
    }
}
