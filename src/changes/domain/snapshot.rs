//! The working tree at one moment, and where HEAD pointed.

use std::collections::HashMap;

/// What a file looked like from the outside: enough to tell it changed, without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ns: i128,
}

/// The working tree at one moment; see the module docs. Two snapshots are equal when
/// the working tree is the same (tree and untracked files), whenever they were taken.
#[derive(Debug, Clone, Eq)]
pub struct Snapshot {
    /// Tree id of the tracked files, as they are in the working tree.
    pub tree: String,
    /// Untracked, non-ignored files by repository-relative path.
    pub untracked: HashMap<String, Stamp>,
    /// Wall-clock time the snapshot began (ns since the epoch; 0 = unknown). A file
    /// that shows up untracked after the turn but is OLDER than this existed before,
    /// hidden by `.gitignore` (the turn un-ignored it): it is never treated as created
    /// by the turn, so undo can never delete it.
    pub taken_ns: i128,
    /// HEAD commit and branch when the snapshot was taken: if either differs between the
    /// two snapshots of a turn, somebody else moved the repository during it.
    pub repo: RepoState,
}

/// Where the repository's HEAD pointed: the commit (`None` before the first commit) and
/// the branch (`None` when detached).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoState {
    pub head: Option<String>,
    pub branch: Option<String>,
}

impl RepoState {
    fn label(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(b), _) => b.clone(),
            (None, Some(h)) => format!("detached at {}", &h[..h.len().min(8)]),
            (None, None) => "no commits".to_string(),
        }
    }
}

/// HEAD (or its branch) moved while the turn ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMove {
    pub from: RepoState,
    pub to: RepoState,
}

impl RepoMove {
    /// `branch switched dashboard → master`, `branch dashboard moved to another commit`.
    pub fn describe(&self) -> String {
        if self.from.branch != self.to.branch {
            format!("branch switched {} → {}", self.from.label(), self.to.label())
        } else if self.from.branch.is_some() {
            format!("branch {} moved to another commit", self.from.label())
        } else {
            format!("HEAD moved: {} → {}", self.from.label(), self.to.label())
        }
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.tree == other.tree && self.untracked == other.untracked
    }
}
