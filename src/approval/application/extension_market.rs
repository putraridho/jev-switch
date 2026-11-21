//! Installing, updating and removing extensions (commands, skills, recipes, themes, mods),
//! with a checksum for every file.
//!
//! An extension is a folder with a `jev-extension.json` manifest next to the files it ships:
//!
//! ```json
//! { "name": "review-kit", "version": "1.2.0", "description": "...",
//!   "files": { "commands/review-pr.md": "<sha-256 hex>", "recipes/fix-tests.md": "<sha-256 hex>" } }
//! ```
//!
//! `pack` writes the manifest from the files; `install` refuses a source whose files do not
//! match it, copies only what the manifest lists, and remembers where it came from so `update`
//! can fetch it again; `list` re-hashes what is installed and says when something was edited.
//! Sources are folders on disk. Hooks are not an extension file type: a hook runs shell
//! commands, so it can only be written by the person themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};


pub const MANIFEST: &str = "jev-extension.json";
const REGISTRY: &str = "extensions.json";
/// The folders an extension may put files in.
const ALLOWED_DIRS: [&str; 5] = ["commands", "skills", "recipes", "themes", "mods"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketError(pub String);

impl std::fmt::Display for MarketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MarketError {}

fn fail<T>(message: impl Into<String>) -> Result<T, MarketError> {
    Err(MarketError(message.into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Path inside the extension to the SHA-256 of that file.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    name: String,
    version: String,
    source: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    extensions: Vec<Record>,
}

fn extensions_dir(user: &Path) -> PathBuf {
    user.join("extensions")
}

/// The folders of the installed extensions, in name order.
pub fn installed_dirs(user: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(extensions_dir(user)) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    dirs.sort();
    dirs
}

fn load_registry(user: &Path) -> Registry {
    std::fs::read_to_string(user.join(REGISTRY)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_registry(user: &Path, registry: &Registry) -> Result<(), MarketError> {
    let text = serde_json::to_string_pretty(registry).map_err(|e| MarketError(e.to_string()))?;
    std::fs::create_dir_all(user).map_err(|e| MarketError(format!("{}: {e}", user.display())))?;
    crate::shared::infrastructure::private_file::write_atomic(&user.join(REGISTRY), &text).map_err(|e| MarketError(format!("cannot save the extension list: {e}")))
}

pub mod installer;
pub mod listing;
pub mod manifest;
mod pristine;
#[cfg(test)]
mod tests;
