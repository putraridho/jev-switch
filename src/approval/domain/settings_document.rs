//! The two lists of Claude Code's settings that jev-code edits, where the files are, and
//! why one could not be read or written. The file format itself is
//! `approval::infrastructure::settings_json`.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where the project's own rules live, relative to the project folder.
pub const LOCAL_SETTINGS: &str = ".claude/settings.local.json";

/// Why a settings file could not be read or written. The text names the file, so it can
/// be shown as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError(pub String);

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SettingsError {}

pub fn local_settings_path(project: &Path) -> PathBuf {
    project.join(LOCAL_SETTINGS)
}

/// The user-level settings Claude Code also reads (`~/.claude/settings.json`).
pub fn user_settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

/// The two lists of the `permissions` object this module edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Allow,
    Directories,
}
