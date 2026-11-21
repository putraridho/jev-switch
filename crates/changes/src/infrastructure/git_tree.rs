//! The git-backed [`WorkingTree`]: the free functions of the infrastructure modules behind the port.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::application::ports::WorkingTree;
use crate::domain::change_set::ChangeSummary;
use crate::domain::snapshot::Snapshot;
use crate::domain::turn_activity::TurnActivity;
use crate::domain::undo_report::UndoError;
use crate::infrastructure::git_output::{git_text, parse_ls_tree_paths};
use crate::infrastructure::git_working_tree as tree;
use crate::infrastructure::undo_files;

/// Git, run in the real working tree.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitWorkingTree;

impl WorkingTree for GitWorkingTree {
    fn no_repo_programs(&self, dir: &Path) -> Vec<String> {
        jev_shared::infrastructure::git_guard::no_repo_programs(dir)
    }

    fn snapshot(&self, dir: &Path) -> Option<Snapshot> {
        tree::snapshot(dir)
    }

    fn summarize_turn(&self, dir: &Path, before: &Snapshot, after: &Snapshot, activity: &TurnActivity) -> Option<ChangeSummary> {
        tree::summarize_turn(dir, before, after, activity)
    }

    fn diff_text(&self, dir: &Path, summary: &ChangeSummary, path: Option<&str>) -> Option<String> {
        tree::diff_text(dir, summary, path)
    }

    fn repo_root(&self, dir: &Path) -> Option<PathBuf> {
        tree::repo_root(dir)
    }

    fn paths_differing(&self, root: &Path, from: &str, to: &str) -> Option<Vec<String>> {
        let out = git_text(root, None, &["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", from, to])?;
        Some(out.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
    }

    fn paths_in_tree(&self, root: &Path, tree: &str, wanted: &[&str]) -> Option<HashSet<String>> {
        let mut found = HashSet::new();
        for chunk in wanted.chunks(200) {
            let mut args = vec!["--literal-pathspecs", "ls-tree", "-z", tree, "--"];
            args.extend(chunk);
            found.extend(parse_ls_tree_paths(&git_text(root, None, &args)?));
        }
        Some(found)
    }

    fn inside_root(&self, root_canon: &Path, target: &Path) -> bool {
        undo_files::inside_root(root_canon, target)
    }

    fn blocking_parent(&self, root: &Path, rel: &str, deleting: &HashSet<&str>) -> Option<String> {
        undo_files::blocking_parent(root, rel, deleting)
    }

    fn folder_in_the_way(&self, root: &Path, rel: &str, deleting: &HashSet<&str>) -> bool {
        undo_files::folder_in_the_way(root, rel, deleting)
    }

    fn restore_paths(&self, root: &Path, tree: &str, paths: &[&str]) -> Result<(), UndoError> {
        undo_files::restore_paths(root, tree, paths)
    }

    fn remove_file_and_empty_parents(&self, root: &Path, rel: &str) -> Result<(), UndoError> {
        undo_files::remove_file_and_empty_parents(root, rel)
    }
}
