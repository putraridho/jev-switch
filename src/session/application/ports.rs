//! The door the use cases have to where sessions are kept: the session files of a project
//! and the marks beside them.
//!
//! Owns the port ([`SessionStore`]) and the one process-wide place its adapter is looked up,
//! the same shape as [`crate::shared::application::host`]. The real adapter is
//! [`crate::session::infrastructure::session_file::FileSessions`]; a test or a binary that
//! wants another calls [`install`] before the first use.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::session::domain::saved_session::{ResumeError, SavedSession, SessionEntry};
use crate::session::domain::session_meta::MetaStore;

/// Where saved sessions live and how they are read, listed, pruned and marked.
pub trait SessionStore: Send + Sync {
    /// The project folder as it is keyed on disk.
    fn project_key(&self, dir: &Path) -> String;
    /// The folder of one project's session files.
    fn project_sessions_dir(&self, state_dir: &Path, project_dir: &Path) -> PathBuf;
    /// A fresh session file name for the project.
    fn new_session_file(&self, state_dir: &Path, project_dir: &Path, now_ms: u64) -> PathBuf;
    /// The most recent session of the project other than `current`.
    fn load_latest(&self, state_dir: &Path, project_dir: &Path, now_ms: u64, current: Option<&Path>) -> Result<(PathBuf, SavedSession), ResumeError>;
    /// Every usable saved session of the project other than `current`, newest first.
    fn list_sessions(&self, state_dir: &Path, project_dir: &Path, now_ms: u64, current: Option<&Path>) -> Vec<SessionEntry>;
    /// Load one session file of the project.
    fn load_path(&self, project_dir: &Path, path: &Path) -> Result<SavedSession, ResumeError>;
    /// Delete the oldest sessions of the project beyond the keep limit, never `current`.
    fn prune(&self, state_dir: &Path, project_dir: &Path, current: &Path);
    /// The marks kept in `state_dir`.
    fn load_marks(&self, state_dir: &Path) -> MetaStore;
    /// Write the marks back.
    fn save_marks(&self, state_dir: &Path, marks: &MetaStore) -> std::io::Result<()>;
}

static INSTALLED: OnceLock<Box<dyn SessionStore>> = OnceLock::new();

/// Use `store` for the rest of the process. `false` when one was already chosen.
pub fn install(store: Box<dyn SessionStore>) -> bool {
    INSTALLED.set(store).is_ok()
}

/// The store in use: the installed one, else the session files on disk.
pub fn get() -> &'static dyn SessionStore {
    &**INSTALLED.get_or_init(|| Box::new(crate::session::infrastructure::session_file::FileSessions))
}
