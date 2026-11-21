//! The file the marks of the sessions live in (`<state>/session-meta.json`), beside the
//! sessions: a damaged meta file never costs a conversation. The marks themselves are
//! [`crate::session::domain::session_meta`].

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::session::domain::session_meta::{Meta, MetaStore};
use crate::shared::infrastructure::private_file::write_atomic;

const FILE: &str = "session-meta.json";

#[derive(Serialize, Deserialize)]
struct MetaWire {
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    archived: bool,
}

impl From<MetaWire> for Meta {
    fn from(value: MetaWire) -> Self {
        Self { tags: value.tags, pinned: value.pinned, archived: value.archived }
    }
}

impl From<&Meta> for MetaWire {
    fn from(value: &Meta) -> Self {
        Self { tags: value.tags.clone(), pinned: value.pinned, archived: value.archived }
    }
}

/// The marks kept in `state_dir`; none when the file is missing or unreadable.
pub fn load(state_dir: &Path) -> MetaStore {
    let entries: BTreeMap<String, MetaWire> = std::fs::read_to_string(state_dir.join(FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    MetaStore::from_entries(entries.into_iter().map(|(key, value)| (key, value.into())).collect())
}

pub fn save(state_dir: &Path, marks: &MetaStore) -> std::io::Result<()> {
    let entries: BTreeMap<_, MetaWire> = marks.entries().iter().map(|(key, value)| (key, value.into())).collect();
    let text = serde_json::to_string_pretty(&entries).map_err(std::io::Error::other)?;
    write_atomic(&state_dir.join(FILE), &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn marks_survive_a_reload_and_blank_ones_are_forgotten() {
        let dir = temp("roundtrip");
        let session = Path::new("/s/abc/100-1.json");
        let mut store = load(&dir);
        store.update(session, |m| {
            m.pinned = true;
            m.tags = vec!["b".into(), "a".into(), "b".into()];
        });
        save(&dir, &store).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "{\n  \"100-1\": {\n    \"tags\": [\n      \"a\",\n      \"b\"\n    ],\n    \"pinned\": true,\n    \"archived\": false\n  }\n}");
        let again = load(&dir);
        assert_eq!(again.get(session), Meta { tags: vec!["a".into(), "b".into()], pinned: true, archived: false });
        let mut again = again;
        again.update(session, |m| *m = Meta::default());
        save(&dir, &again).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap().trim(), "{}");
    }

    #[test]
    fn a_damaged_file_means_no_marks_not_an_error() {
        let dir = temp("damaged");
        std::fs::write(dir.join(FILE), "{ nope").unwrap();
        assert!(!load(&dir).is_pinned(Path::new("x/1-1.json")));
    }
}
