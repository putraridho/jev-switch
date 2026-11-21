//! What `/permissions` shows.

use std::path::PathBuf;

/// What `/permissions` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub local_path: PathBuf,
    pub allow: Vec<String>,
    pub directories: Vec<String>,
    pub user_path: Option<PathBuf>,
    pub user_allow: Vec<String>,
    /// Files that could not be read, in words.
    pub problems: Vec<String>,
}
