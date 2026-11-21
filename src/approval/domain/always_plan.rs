//! What the `a` choice would save for one request, and how a question ended.

use std::path::{Component, PathBuf};

use crate::approval::domain::tool_input::ToolInput;
use crate::approval::domain::grantable::{grantable_folder, Places};
use crate::approval::domain::refusal::Refusal;
use crate::approval::domain::rule_policy::{literal_path, rules_for};
use crate::approval::domain::settings_document::SettingsError;

/// What "always" would save for one request: an allow rule, a folder Claude Code may then
/// use, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Always {
    /// The allow rules to save: one, or one per simple command of a chain
    /// (`cargo build 2>&1 | grep x` saves `Bash(cargo build:*)` and `Bash(grep:*)`).
    pub rules: Vec<String>,
    pub folder: Option<PathBuf>,
}

impl Always {
    /// What is saved, in words that are also what the answer line says.
    pub fn label(&self) -> String {
        let folder = self.folder.as_ref().map(|f| format!("folder {}", f.display()));
        let rules = (!self.rules.is_empty()).then(|| self.rules.join(", "));
        match (rules, folder) {
            (Some(rules), Some(folder)) => format!("{rules} and {folder}"),
            (Some(rules), None) => rules,
            (None, Some(folder)) => folder,
            (None, None) => String::new(),
        }
    }
}

/// What the `a` choice is for one request, as the engine decided it: what it would save, or
/// why it is not offered. The screen only shows this; it never derives or saves anything.
pub type Offer = Result<Always, Refusal>;

/// The folder a call that was asked about because of `blocked_path` needs allowed: the path
/// itself when it is a folder, else its parent. Refused when it is not one definite absolute
/// place, or when [`grantable_folder`] says no.
pub(super) fn blocked_folder(blocked_path: &str, places: &Places) -> Result<PathBuf, Refusal> {
    let path = literal_path(blocked_path)?;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::CurDir)) {
        return Err(Refusal::UnsafePath);
    }
    let dir = if places.disk.0.is_dir(path) { path } else { path.parent().ok_or(Refusal::FolderTooBroad)? };
    grantable_folder(dir, places)?;
    Ok(dir.to_path_buf())
}

/// What the `a` choice would save, or why it is not offered. A Bash call that was asked about
/// because its path is outside the allowed folders needs that folder allowed (a command rule
/// alone would be asked about again), with the command rule too when a safe one exists.
pub fn always_plan(tool: &str, input: &ToolInput, blocked_path: Option<&str>, places: &Places) -> Offer {
    let rules = rules_for(tool, input, blocked_path, places);
    match (tool, blocked_path) {
        ("Bash", Some(path)) => Ok(Always { rules: rules.unwrap_or_default(), folder: Some(blocked_folder(path, places)?) }),
        _ => Ok(Always { rules: rules?, folder: None }),
    }
}

/// How one approval question ended, as the engine settled it AFTER saving: the screen shows
/// exactly this, so it can never say "always" for something that was not saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Allowed this once.
    Allowed,
    /// Allowed, and what the offer named is now in the project's settings.
    Remembered(Always),
    /// "Always" was chosen but the settings file could not be written: allowed this once,
    /// nothing saved.
    NotSaved(SettingsError),
    /// Denied, with the reason the model is told.
    Denied(String),
    /// The person interrupted the question (Ctrl-C): the turn stops.
    Stopped,
}
