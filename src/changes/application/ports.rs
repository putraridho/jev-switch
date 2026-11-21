//! What the change use cases need from the outside: a working tree to snapshot, diff and
//! put files back in. The git-backed adapter is `changes::infrastructure::git_tree`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::changes::domain::change_set::ChangeSummary;
use crate::changes::domain::snapshot::Snapshot;
use crate::changes::domain::turn_activity::TurnActivity;
use crate::changes::domain::undo_report::UndoError;

/// The working tree a turn changes, and the repository around it. Every method blocks.
pub trait WorkingTree: Send + Sync {
    /// The state of the tree at `dir`; `None` outside a repository or when git fails.
    fn snapshot(&self, dir: &Path) -> Option<Snapshot>;
    /// What differs between two snapshots, with the turn's own activity told apart from outside edits.
    fn summarize_turn(&self, dir: &Path, before: &Snapshot, after: &Snapshot, activity: &TurnActivity) -> Option<ChangeSummary>;
    /// The text diff of `summary`, whole or for one repository-relative path.
    fn diff_text(&self, dir: &Path, summary: &ChangeSummary, path: Option<&str>) -> Option<String>;
    /// The repository root `dir` belongs to.
    fn repo_root(&self, dir: &Path) -> Option<PathBuf>;

    /// The paths that differ between trees `from` and `to` of the repository at `root`.
    fn paths_differing(&self, root: &Path, from: &str, to: &str) -> Option<Vec<String>>;
    /// Of `wanted`, the paths that are regular files or links in `tree` (submodules left out).
    fn paths_in_tree(&self, root: &Path, tree: &str, wanted: &[&str]) -> Option<HashSet<String>>;
    /// True when `target` really lies inside `root_canon`, looking through symlinks.
    fn inside_root(&self, root_canon: &Path, target: &Path) -> bool;
    /// Something that is not a folder where a folder must be, in the way of `rel`'s parents.
    fn blocking_parent(&self, root: &Path, rel: &str, deleting: &HashSet<&str>) -> Option<String>;
    /// A folder where the file `rel` has to go, holding something the undo does not delete.
    fn folder_in_the_way(&self, root: &Path, rel: &str, deleting: &HashSet<&str>) -> bool;
    /// Put `paths` back as they are in `tree`.
    fn restore_paths(&self, root: &Path, tree: &str, paths: &[&str]) -> Result<(), UndoError>;
    /// Delete `rel`, and the directories the turn created for it that are now empty.
    fn remove_file_and_empty_parents(&self, root: &Path, rel: &str) -> Result<(), UndoError>;
}
