//! What an undo did, and why it refused.

use std::fmt;

/// What an undo did, by repository-relative path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoReport {
    /// Modified files put back to their old content.
    pub restored: Vec<String>,
    /// Files the turn added, now deleted.
    pub removed: Vec<String>,
    /// Files the turn deleted (or renamed away), now back.
    pub recreated: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoError {
    /// There is no previous turn, or it changed nothing, or it was already undone.
    NothingToUndo,
    /// These files differ from how the turn left them: the user or a later turn edited
    /// them again. Nothing was touched.
    ChangedSince(Vec<String>),
    /// The turn changed these untracked files that already existed. There is no earlier
    /// copy to put back, so nothing was touched.
    UntrackedModified(Vec<String>),
    /// HEAD or the branch moved (during the turn, or since): the turn's changes cannot be
    /// told apart from someone else's, so nothing was touched.
    RepoMoved(String),
    /// git named a path an undo must not touch (outside the repository, or `.git`).
    UnsafePath(String),
    /// git or the file system failed.
    Failed(String),
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToUndo => f.write_str("there is nothing to undo (no previous turn changed any files)"),
            Self::ChangedSince(files) => write!(
                f,
                "not undone: {} changed since that turn, so undoing would lose those edits: {}",
                if files.len() == 1 { "a file was" } else { "files were" },
                files.join(", ")
            ),
            Self::UntrackedModified(files) => write!(
                f,
                "not undone: the turn changed {} not tracked by git, so there is no earlier copy to put back: {}. Nothing was touched; \
                 revert {} by hand (files you `git add` before a turn can be undone)",
                if files.len() == 1 { "a file that is" } else { "files that are" },
                files.join(", "),
                if files.len() == 1 { "it" } else { "them" }
            ),
            Self::RepoMoved(what) => write!(
                f,
                "not undone: {what}, so this turn's changes cannot be told apart from changes made by others. Nothing was touched; \
                 revert by hand if you need to"
            ),
            Self::UnsafePath(p) => write!(f, "not undone: refusing to touch `{p}`"),
            Self::Failed(m) => write!(f, "undo failed: {m}"),
        }
    }
}

impl std::error::Error for UndoError {}
