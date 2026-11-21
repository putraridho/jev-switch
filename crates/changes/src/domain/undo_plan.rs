//! What an undo does, planned from the summary before anything is touched.

use std::collections::{HashMap, HashSet};

use crate::domain::file_change::{ChangeStatus, FileChange};
use crate::domain::safe_path::is_safe_rel_path;
use crate::domain::snapshot::Stamp;
use crate::domain::undo_report::UndoError;

/// One thing an undo does to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate) enum Step {
    /// The turn created this path.
    Delete(String),
    /// Put the path back to its `before` content; `recreated` is whether the turn deleted it.
    Restore { path: String, recreated: bool },
}

impl Step {
    pub(in crate) fn path(&self) -> &str {
        match self {
            Self::Delete(p) | Self::Restore { path: p, .. } => p,
        }
    }
}

/// What to do to undo `files`; refuses on the first unsafe path, and on untracked files
/// that cannot be put back, before anything runs.
pub(in crate) fn plan_undo(files: &[FileChange]) -> Result<Vec<Step>, UndoError> {
    let mut steps = Vec::new();
    let mut untracked = Vec::new();
    for f in files {
        let paths = std::iter::once(&f.path).chain(f.old_path.as_ref());
        if let Some(bad) = paths.into_iter().find(|p| !is_safe_rel_path(p)) {
            return Err(UndoError::UnsafePath(bad.clone()));
        }
        match (f.status, &f.old_path) {
            (ChangeStatus::Added, _) => steps.push(Step::Delete(f.path.clone())),
            (ChangeStatus::Modified, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: false }),
            (ChangeStatus::Deleted, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: true }),
            (ChangeStatus::ModifiedUntracked, _) => untracked.push(f.path.clone()),
            (ChangeStatus::Renamed, old) => {
                steps.push(Step::Delete(f.path.clone()));
                if let Some(old) = old {
                    steps.push(Step::Restore { path: old.clone(), recreated: true });
                }
            }
        }
    }
    if !untracked.is_empty() {
        return Err(UndoError::UntrackedModified(untracked));
    }
    if steps.is_empty() {
        return Err(UndoError::NothingToUndo);
    }
    Ok(steps)
}

/// The paths an undo would overwrite or delete although someone changed them after the
/// turn. `tree_diff` are the tracked paths that differ between the tree the turn left and
/// the one now; `now` is the untracked manifest now. An untracked file the turn added is
/// fine to delete only while it still looks as the turn left it; any other untracked file
/// sitting at a path the undo is about to write is somebody's.
pub(in crate) fn changed_since(steps: &[Step], added: &HashMap<String, Stamp>, tree_diff: &HashSet<&str>, now: &HashMap<String, Stamp>) -> Vec<String> {
    steps
        .iter()
        .map(Step::path)
        .filter(|p| {
            tree_diff.contains(p)
                || match added.get(*p) {
                    Some(left) => now.get(*p) != Some(left),
                    None => now.contains_key(*p),
                }
        })
        .map(String::from)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_undo_plan_covers_each_kind_and_refuses_unsafe_paths_up_front() {
        let fc = |path: &str, status, old: Option<&str>| FileChange { path: path.into(), status, added: 0, removed: 0, old_path: old.map(String::from) };
        let plan = plan_undo(&[
            fc("m", ChangeStatus::Modified, None),
            fc("a", ChangeStatus::Added, None),
            fc("d", ChangeStatus::Deleted, None),
            fc("new", ChangeStatus::Renamed, Some("old")),
        ])
        .unwrap();
        assert_eq!(
            plan,
            vec![
                Step::Restore { path: "m".into(), recreated: false },
                Step::Delete("a".into()),
                Step::Restore { path: "d".into(), recreated: true },
                Step::Delete("new".into()),
                Step::Restore { path: "old".into(), recreated: true },
            ]
        );
        assert_eq!(plan_undo(&[]), Err(UndoError::NothingToUndo));
        let err = plan_undo(&[fc("ok", ChangeStatus::Added, None), fc("../../etc/passwd", ChangeStatus::Modified, None)]);
        assert_eq!(err, Err(UndoError::UnsafePath("../../etc/passwd".into())));
        let err = plan_undo(&[fc("new", ChangeStatus::Renamed, Some("/abs"))]);
        assert_eq!(err, Err(UndoError::UnsafePath("/abs".into())));
    }

    #[test]
    fn the_edited_since_rule_covers_added_untracked_tracked_and_recreated_paths() {
        let st = |n| Stamp { size: n, mtime_ns: 0 };
        let steps = vec![
            Step::Delete("same".into()),
            Step::Delete("touched".into()),
            Step::Delete("missing".into()),
            Step::Restore { path: "tracked-edit".into(), recreated: false },
            Step::Restore { path: "recreated".into(), recreated: true },
            Step::Restore { path: "fine".into(), recreated: true },
        ];
        let added: HashMap<_, _> = [("same", st(1)), ("touched", st(1)), ("missing", st(1))].map(|(p, s)| (p.to_string(), s)).into();
        let now: HashMap<_, _> = [("same", st(1)), ("touched", st(2)), ("recreated", st(1))].map(|(p, s)| (p.to_string(), s)).into();
        let diff: HashSet<&str> = ["tracked-edit"].into();
        assert_eq!(changed_since(&steps, &added, &diff, &now), ["touched", "missing", "tracked-edit", "recreated"]);
    }
}
