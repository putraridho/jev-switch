//! The door the use cases have to where sessions are kept: the session files of a project
//! and the marks beside them.
//!
//! Owns the port ([`SessionStore`]). Use cases receive it through their constructor or as a
//! parameter; the composition roots pass the real adapter,
//! [`crate::session::infrastructure::session_file::FileSessions`], and tests pass their own.

use std::path::{Path, PathBuf};

use crate::session::domain::saved_session::{ResumeError, SavedSession, SessionEntry};
use crate::session::domain::session_meta::MetaStore;

/// Where saved sessions live and how they are read, listed, pruned and marked.
pub trait SessionStore: Send + Sync {
    /// Render a session in the saved-session format.
    fn render_session(&self, saved: &SavedSession) -> String;
    /// Parse a saved-session file.
    fn parse_session(&self, text: &str) -> Result<SavedSession, String>;
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
