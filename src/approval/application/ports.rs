//! The doors approval's use cases have to Claude Code's settings files. Real adapter:
//! [`crate::approval::infrastructure::claude_settings_file::ClaudeSettingsFiles`].

use std::path::Path;

use crate::approval::domain::always_plan::Always;
use crate::approval::domain::grantable::Places;
use crate::approval::domain::listing::Listing;
use crate::approval::domain::settings_document::SettingsError;
use crate::approval::domain::tool_input::ToolInput;

/// The rules a person already saved, and saving an "always" answer: what a turn's approver
/// needs.
pub trait RememberedRules: Send + Sync {
    /// The rules that already allow this call, when every one of them was saved before
    /// (project or user settings); `None` when the person has to be asked.
    fn remembered(&self, tool: &str, input: &ToolInput, blocked_path: Option<&str>, places: &Places) -> Option<Vec<String>>;
    /// Save what `always` says into the project's local settings, rule and folder in one
    /// write. False when everything was already there.
    fn save_always(&self, project: &Path, always: &Always) -> Result<bool, SettingsError>;
}

/// What `/permissions` edits and lists in the project's local settings.
pub trait PermissionsEditor: Send + Sync {
    /// Add an allow rule. False when it was already there.
    fn add_allow_rule(&self, project: &Path, rule: &str) -> Result<bool, SettingsError>;
    /// Remove an allow rule. False when it was not there.
    fn remove_allow_rule(&self, project: &Path, rule: &str) -> Result<bool, SettingsError>;
    /// Add an additional directory. False when it was already there.
    fn add_directory(&self, project: &Path, dir: &Path) -> Result<bool, SettingsError>;
    /// The project's rules and folders and, read-only, the user-level allow list.
    fn listing(&self, project: &Path, home: Option<&Path>) -> Listing;
}
