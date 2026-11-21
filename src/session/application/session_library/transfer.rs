//! Export a session to a file and import it back, into any project.

use std::path::{Path, PathBuf};

use super::*;
use crate::session::domain::saved_session::SavedSession;
use crate::session::domain::session::Sessions;
use crate::session::infrastructure::session_file;
use crate::shared::application::host::write_private as write_atomic;

// ----------------------------------------------------------- export / import

/// The session as a file anyone can carry to another machine: the saved-session format
/// itself, which `import` reads back. Secrets are masked in the exported copy.
pub fn export_to(entry: &LibraryEntry, dest: &Path) -> std::io::Result<()> {
    if dest.exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} exists already", dest.display())));
    }
    let mut value = entry.saved.to_json();
    redact_export(&mut value);
    write_atomic(dest, &value.to_string())
}

/// Mask strings before serializing, so escaping cannot hide secrets or break JSON.
fn redact_export(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = crate::shared::domain::redact::redact(text),
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_export),
        serde_json::Value::Object(values) => values.values_mut().for_each(redact_export),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Unreadable(String),
    NotASession(String),
    Empty,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(m) => write!(f, "cannot read the file: {m}"),
            Self::NotASession(m) => write!(f, "that is not a jev session file: {m}"),
            Self::Empty => f.write_str("the session has no turns"),
        }
    }
}

/// Add an exported session to `project_dir`'s sessions (it becomes resumable there, and
/// keeps its time). Returns the new session file.
pub fn import_file(state_dir: &Path, project_dir: &Path, source: &Path, now_ms: u64) -> Result<PathBuf, ImportError> {
    let text = std::fs::read_to_string(source).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let saved = SavedSession::parse(&text).map_err(ImportError::NotASession)?;
    adopt(state_dir, project_dir, saved, now_ms)
}

/// Store `saved` as a session of `project_dir`.
pub fn adopt(state_dir: &Path, project_dir: &Path, mut saved: SavedSession, now_ms: u64) -> Result<PathBuf, ImportError> {
    if saved.sessions.turns() == 0 {
        return Err(ImportError::Empty);
    }
    saved.project = session_file::project_key(project_dir);
    // Another machine's CLI sessions mean nothing here.
    saved.sessions = strip_cli_ids(saved.sessions);
    let path = session_file::new_session_file(state_dir, project_dir, now_ms);
    write_atomic(&path, &saved.render()).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    Ok(path)
}

/// The same dialogue with no provider CLI session ids and nobody marked as caught up, so
/// the first turn on any provider is handed the whole conversation.
fn strip_cli_ids(old: Sessions) -> Sessions {
    let mut fresh = Sessions::new();
    fresh.set_name(old.name());
    for turn in old.log() {
        fresh.push_unseen(turn.clone());
    }
    fresh
}
