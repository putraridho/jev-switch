//! A Claude Code settings document and the two lists of it that jev-code edits.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

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

impl List {
    fn key(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Directories => "additionalDirectories",
        }
    }
}

/// A settings document: any JSON object, with the shape of the part we edit checked.
/// Everything else in it is carried along untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    doc: Value,
}

impl Settings {
    /// An empty document, for a file that does not exist yet.
    pub fn empty() -> Self {
        Self { doc: Value::Object(Map::new()) }
    }

    /// Parse `text`. Refuses anything that would have to be overwritten to be edited:
    /// invalid JSON, a root that is not an object, `permissions` that is not an object,
    /// or one of our two lists that is not a list of strings.
    pub fn parse(text: &str, file: &Path) -> Result<Self, SettingsError> {
        let err = |why: String| SettingsError(format!("{}: {why}; left as it is", file.display()));
        let doc: Value = serde_json::from_str(text).map_err(|e| err(format!("not valid JSON ({e})")))?;
        let Some(root) = doc.as_object() else { return Err(err("the top level is not a JSON object".into())) };
        if let Some(permissions) = root.get("permissions") {
            let Some(permissions) = permissions.as_object() else { return Err(err("\"permissions\" is not an object".into())) };
            for list in [List::Allow, List::Directories] {
                let ok = permissions
                    .get(list.key())
                    .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string)));
                if !ok {
                    return Err(err(format!("\"permissions.{}\" is not a list of strings", list.key())));
                }
            }
        }
        Ok(Self { doc })
    }

    /// The entries of one list, in file order.
    pub fn list(&self, list: List) -> Vec<String> {
        self.doc["permissions"][list.key()]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    fn entries_mut(&mut self, list: List) -> &mut Vec<Value> {
        let root = self.doc.as_object_mut().expect("checked in parse");
        let permissions = root.entry("permissions").or_insert_with(|| json!({}));
        let permissions = permissions.as_object_mut().expect("checked in parse");
        permissions.entry(list.key()).or_insert_with(|| json!([])).as_array_mut().expect("checked in parse")
    }

    /// Add `entry` unless it is there. True when the document changed.
    pub fn add(&mut self, list: List, entry: &str) -> bool {
        if self.list(list).iter().any(|e| e == entry) {
            return false;
        }
        self.entries_mut(list).push(Value::String(entry.to_string()));
        true
    }

    /// Remove every copy of `entry`. True when the document changed. An emptied list is
    /// left as an empty list, never deleted: the user may have put it there.
    pub fn remove(&mut self, list: List, entry: &str) -> bool {
        if !self.list(list).iter().any(|e| e == entry) {
            return false;
        }
        self.entries_mut(list).retain(|v| v.as_str() != Some(entry));
        true
    }

    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.doc).unwrap_or_else(|_| "{}".into());
        text.push('\n');
        text
    }
}
