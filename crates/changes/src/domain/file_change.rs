//! One file a turn changed, and how.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// An untracked file that existed before the turn and was changed or removed by it.
    /// There is no earlier copy, so it is reported but never undone.
    ModifiedUntracked,
}

/// One file the turn changed. Paths are relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The path after the turn (for a rename, the new one).
    pub path: String,
    pub status: ChangeStatus,
    /// Lines added and removed; 0 for binary files.
    pub added: u32,
    pub removed: u32,
    /// For [`ChangeStatus::Renamed`], where the file was before.
    pub old_path: Option<String>,
}

impl FileChange {
    pub(super) fn new(path: String, status: ChangeStatus) -> Self {
        Self { path, status, added: 0, removed: 0, old_path: None }
    }
}
