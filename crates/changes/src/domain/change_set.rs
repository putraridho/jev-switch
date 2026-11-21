//! What a turn changed: the summary, what others changed meanwhile, and who did what.

use std::collections::{HashMap, HashSet};

use crate::domain::file_change::FileChange;
use crate::domain::snapshot::{RepoMove, RepoState, Stamp};

/// Changes the working tree went through during the turn that the turn did not make
/// (as far as can be told). Shown as a warning, never undone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutsideChanges {
    pub files: Vec<FileChange>,
    /// Set when HEAD or the branch moved during the turn.
    pub moved: Option<RepoMove>,
    /// A shell command (or a write to an unknown path) ran, so files in `ChangeSummary::files`
    /// that the turn's own edit tools did not name may still be someone else's.
    pub uncertain: bool,
}

impl OutsideChanges {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.moved.is_none() && !self.uncertain
    }

    /// The one warning line for the end of a turn; `None` when there is nothing to warn about.
    pub fn warning(&self) -> Option<String> {
        let n = self.files.len();
        let files = if n == 1 { "1 file".to_string() } else { format!("{n} files") };
        match (&self.moved, n) {
            (Some(m), 1..) => Some(format!("{files} changed outside this turn ({}); not included, /undo won't touch them", m.describe())),
            (Some(m), 0) => Some(format!("{}; /undo is disabled for this turn", m.describe())),
            (None, 1..) => Some(format!("{files} changed outside this turn (another process); not included, /undo won't touch them")),
            (None, 0) if self.uncertain => Some("a shell command ran, so the files listed may include changes from other processes".to_string()),
            (None, 0) => None,
        }
    }
}

/// The difference between two working-tree snapshots. `files` is never empty in a
/// summary the engine keeps: a turn that changed nothing has no summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSummary {
    pub files: Vec<FileChange>,
    /// Tree ids of the tracked files before and after the turn.
    pub before: String,
    pub after: String,
    /// The untracked files the turn added, as they were when it ended: undo deletes one
    /// only while it still looks like this.
    pub added_stamps: HashMap<String, Stamp>,
    /// HEAD and branch when the turn ended; `/undo` refuses if they are different later.
    pub repo_after: RepoState,
    /// What changed that this turn did not do; see [`OutsideChanges`].
    pub outside: OutsideChanges,
}

impl ChangeSummary {
    /// Nothing to show: no attributable file and nothing to warn about.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.outside.is_empty()
    }

    /// Total lines (added, removed) over all files.
    pub fn totals(&self) -> (u32, u32) {
        self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
    }
}

/// Split `files` (everything that differs between the two snapshots) into what this turn
/// did and what somebody else did, given what the turn's tools reported.
///
/// * HEAD or the branch moved: the diff is untrustworthy. Only files the edit tools named
///   stay (and none at all if something opaque ran); `/undo` is refused separately.
/// * a shell command ran: every file stays, marked uncertain unless the tools named them all;
/// * otherwise only the files the edit tools named stay.
pub(in crate) fn attribute(files: Vec<FileChange>, written: &HashSet<String>, opaque: bool, moved: Option<RepoMove>) -> (Vec<FileChange>, OutsideChanges) {
    let named = |f: &FileChange| written.contains(&f.path) || f.old_path.as_ref().is_some_and(|o| written.contains(o));
    if opaque && moved.is_none() {
        let uncertain = files.iter().any(|f| !named(f));
        return (files, OutsideChanges { files: Vec::new(), moved: None, uncertain });
    }
    let (mine, outside): (Vec<FileChange>, Vec<FileChange>) = if opaque { (Vec::new(), files) } else { files.into_iter().partition(named) };
    (mine, OutsideChanges { files: outside, moved, uncertain: false })
}
