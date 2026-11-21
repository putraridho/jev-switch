//! Undo what a turn did, in three named steps: plan it, verify nothing changed since, apply it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::domain::change_set::ChangeSummary;
use crate::domain::snapshot::RepoMove;
use crate::domain::undo_plan::{changed_since, plan_undo, Step};
use crate::domain::undo_report::{UndoError, UndoReport};
use crate::application::ports::WorkingTree;

/// What step 1 decided: the steps to take, and the repository they are taken in.
struct Plan {
    steps: Vec<Step>,
    root: PathBuf,
    root_canon: PathBuf,
}

impl Plan {
    /// The paths that are put back from the old tree.
    fn wanted(&self) -> Vec<&str> {
        self.steps.iter().filter(|&s| matches!(s, Step::Restore { .. })).map(|s| s.path()).collect()
    }

    /// The paths this undo deletes.
    fn deleting(&self) -> HashSet<&str> {
        self.steps.iter().filter(|&s| matches!(s, Step::Delete(_))).map(|s| s.path()).collect()
    }
}

/// Undo what `summary` records, in the repository `dir` belongs to. Blocking.
///
/// Safe by construction: nothing is touched unless every file the turn changed still
/// matches how the turn left it, every path is inside the repository, nothing the turn
/// changed is an untracked file that existed before, and all old contents are known to be
/// in the old tree. Only the paths in the summary are written.
pub fn undo(tree: &dyn WorkingTree, dir: &Path, summary: &ChangeSummary) -> Result<UndoReport, UndoError> {
    let plan = plan(tree, dir, summary)?;
    verify_nothing_changed_since(tree, dir, summary, &plan)?;
    apply(tree, summary, &plan)
}

/// Step 1: refuse a repository that moved, plan the work, and find the repository.
fn plan(tree: &dyn WorkingTree, dir: &Path, summary: &ChangeSummary) -> Result<Plan, UndoError> {
    if let Some(m) = &summary.outside.moved {
        return Err(UndoError::RepoMoved(format!("the repository moved during the turn ({})", m.describe())));
    }
    let steps = plan_undo(&summary.files)?;
    let root = tree.repo_root(dir).ok_or_else(|| UndoError::Failed("not inside a git repository".into()))?;
    let root_canon = std::fs::canonicalize(&root).map_err(|e| UndoError::Failed(format!("cannot resolve {}: {e}", root.to_string_lossy())))?;
    Ok(Plan { steps, root, root_canon })
}

/// Step 2: touch nothing, and say no unless it is still safe.
fn verify_nothing_changed_since(tree: &dyn WorkingTree, dir: &Path, summary: &ChangeSummary, plan: &Plan) -> Result<(), UndoError> {
    let Plan { steps, root, root_canon } = plan;

    // 1. Has anything the turn touched been edited since? Compare against the state the
    // turn left, not against `before`: only a difference from `after` is someone's edit.
    let now = tree.snapshot(dir).ok_or_else(|| UndoError::Failed("could not read the working tree".into()))?;
    if now.repo != summary.repo_after {
        let m = RepoMove { from: summary.repo_after.clone(), to: now.repo.clone() };
        return Err(UndoError::RepoMoved(format!("the repository moved since the turn ({})", m.describe())));
    }
    let differing = tree.paths_differing(root, &summary.after, &now.tree).ok_or_else(|| UndoError::Failed("could not compare the working tree".into()))?;
    let differing: HashSet<&str> = differing.iter().map(String::as_str).collect();
    let changed = changed_since(steps, &summary.added_stamps, &differing, &now.untracked);
    if !changed.is_empty() {
        return Err(UndoError::ChangedSince(changed));
    }

    // 2. Every old content must be in the old tree, so a failure here leaves the tree untouched.
    let wanted = plan.wanted();
    let restorable = tree.paths_in_tree(root, &summary.before, &wanted).ok_or_else(|| UndoError::Failed("could not read the old tree".into()))?;
    if let Some(path) = wanted.iter().find(|p| !restorable.contains(**p)) {
        return Err(UndoError::Failed(format!("{path} is not in the tree from before the turn (or is a submodule)")));
    }

    // 3. Every target must really be inside the repository and not blocked by something
    // that is not ours to replace.
    let deleting = plan.deleting();
    for step in steps {
        if !tree.inside_root(root_canon, &root.join(step.path())) {
            return Err(UndoError::UnsafePath(step.path().to_string()));
        }
    }
    for path in &wanted {
        if let Some(p) = tree.blocking_parent(root, path, &deleting) {
            return Err(UndoError::Failed(format!("{p} is in the way of {path}")));
        }
        if tree.folder_in_the_way(root, path, &deleting) {
            return Err(UndoError::Failed(format!("{path} is now a folder with other things in it")));
        }
    }
    Ok(())
}

/// Step 3: do it. Deletions first, so a rename back never meets its own leftover.
fn apply(tree: &dyn WorkingTree, summary: &ChangeSummary, plan: &Plan) -> Result<UndoReport, UndoError> {
    let Plan { steps, root, .. } = plan;
    let wanted = plan.wanted();
    let mut report = UndoReport::default();
    for step in steps {
        if let Step::Delete(path) = step {
            tree.remove_file_and_empty_parents(root, path)?;
            report.removed.push(path.clone());
        }
    }
    for path in &wanted {
        if tree.folder_in_the_way(root, path, &HashSet::new()) {
            return Err(UndoError::Failed(format!("{path} is now a folder with other things in it")));
        }
    }
    tree.restore_paths(root, &summary.before, &wanted)?;
    for step in steps {
        if let Step::Restore { path, recreated } = step {
            if *recreated { &mut report.recreated } else { &mut report.restored }.push(path.clone());
        }
    }
    Ok(report)
}
