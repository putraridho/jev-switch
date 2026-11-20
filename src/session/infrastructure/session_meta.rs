//! What the person says about a saved session: tags, pinned, archived. Kept beside the
//! sessions, not inside them (`<state>/session-meta.json`), so a session file stays exactly
//! what the engine wrote and a damaged meta file never costs a conversation. Entries are
//! keyed by the session file's name without the extension.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::domain::catch_up::write_atomic;

const FILE: &str = "session-meta.json";

/// One session's marks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub tags: Vec<String>,
    /// Kept at the top of lists and never deleted by the keep-ten rule or `/sessions clean`.
    #[serde(default)]
    pub pinned: bool,
    /// Left out of `/resume` and of the default listing; searchable with `archived`.
    #[serde(default)]
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

#[derive(Debug, Clone, Default)]
pub struct MetaStore {
    file: Option<PathBuf>,
    entries: BTreeMap<String, Meta>,
}

impl MetaStore {
    /// The marks kept in `state_dir`; none when the file is missing or unreadable.
    pub fn load(state_dir: &Path) -> Self {
        let file = state_dir.join(FILE);
        let entries = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { file: Some(file), entries }
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

    pub fn save(&self) -> std::io::Result<()> {
        let Some(file) = &self.file else { return Ok(()) };
        let text = serde_json::to_string_pretty(&self.entries).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tags_are_one_clean_word() {
        assert_eq!(clean_tag("#Bug-Fix"), Some("bug-fix".into()));
        assert_eq!(clean_tag("two words"), None);
        assert_eq!(clean_tag("  "), None);
        assert_eq!(clean_tag(&"x".repeat(40)), None);
    }

    #[test]
    fn marks_survive_a_reload_and_blank_ones_are_forgotten() {
        let dir = temp("roundtrip");
        let session = Path::new("/s/abc/100-1.json");
        let mut store = MetaStore::load(&dir);
        store.update(session, |m| {
            m.pinned = true;
            m.tags = vec!["b".into(), "a".into(), "b".into()];
        });
        store.save().unwrap();
        let again = MetaStore::load(&dir);
        assert_eq!(again.get(session), Meta { tags: vec!["a".into(), "b".into()], pinned: true, archived: false });
        let mut again = again;
        again.update(session, |m| *m = Meta::default());
        again.save().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap().trim(), "{}");
    }

    #[test]
    fn a_damaged_file_means_no_marks_not_an_error() {
        let dir = temp("damaged");
        std::fs::write(dir.join(FILE), "{ nope").unwrap();
        assert!(!MetaStore::load(&dir).is_pinned(Path::new("x/1-1.json")));
    }
}
