//! Tags, pin and archive marks on a session.

use std::path::Path;

use crate::session::infrastructure::session_meta::MetaStore;

// ------------------------------------------------------------------ marks

/// A change to one session's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Pin(bool),
    Archive(bool),
    Tag(String),
    Untag(String),
}

/// Apply `mark` to the session file and save the marks.
pub fn mark(state_dir: &Path, session: &Path, mark: &Mark) -> std::io::Result<()> {
    let mut store = MetaStore::load(state_dir);
    store.update(session, |m| match mark {
        Mark::Pin(on) => m.pinned = *on,
        Mark::Archive(on) => m.archived = *on,
        Mark::Tag(t) => m.tags.push(t.clone()),
        Mark::Untag(t) => m.tags.retain(|x| x != t),
    });
    store.save()
}
