//! Fold the manifests of untracked files into the tracked file list.

use std::collections::{HashMap, HashSet};

use crate::domain::file_change::{ChangeStatus, FileChange};
use crate::domain::snapshot::{Snapshot, Stamp};

/// How much older than the snapshot a "new" file may look before it counts as having
/// existed already (filesystem timestamp granularity and clock skew).
const PRE_EXISTING_SLACK_NS: i128 = 2_000_000_000;

/// The tracked diff of two snapshots combined with what their manifests say.
pub(in crate) struct Merged {
    pub(in crate) files: Vec<FileChange>,
    /// The untracked files the turn added, with their stamps now.
    pub(in crate) added: HashMap<String, Stamp>,
}

/// The untracked paths an addition pass splits into added and revealed, with their stamps.
type StampedPaths<'a> = (Vec<(&'a String, &'a Stamp)>, Vec<(&'a String, &'a Stamp)>);

/// Fold the untracked manifests into the tracked file list.
///
/// * untracked now, not before: `Added` (unless it is a tracked file that left the index,
///   which the tree diff already reports as deleted, and which is still there);
/// * untracked before and changed or gone now: `ModifiedUntracked`;
/// * a pre-existing untracked file the turn only staged shows as added in the tree diff:
///   that is dropped (and caught above if its content changed), so an undo never deletes
///   a file that was already there.
///
/// `stamp_now` looks a path up on disk, for files that are no longer in the manifest.
pub(in crate) fn merge_untracked(tree_files: Vec<FileChange>, before: &Snapshot, after: &Snapshot, stamp_now: impl Fn(&str) -> Option<Stamp>) -> Merged {
    let deleted: HashSet<String> = tree_files.iter().filter(|f| f.status == ChangeStatus::Deleted).map(|f| f.path.clone()).collect();
    let mut files = Vec::new();
    for f in tree_files {
        let existed_untracked = before.untracked.contains_key(&f.path);
        match f.status {
            ChangeStatus::Deleted if after.untracked.contains_key(&f.path) => {}
            ChangeStatus::Added if existed_untracked => {}
            ChangeStatus::Renamed if existed_untracked => {
                if let Some(old) = f.old_path {
                    files.push(FileChange::new(old, ChangeStatus::Deleted));
                }
            }
            _ => files.push(f),
        }
    }

    let mut modified: Vec<&String> = before
        .untracked
        .iter()
        .filter(|(p, was)| after.untracked.get(*p).copied().or_else(|| stamp_now(p)) != Some(**was))
        .map(|(p, _)| p)
        .collect();
    modified.sort();
    files.extend(modified.into_iter().map(|p| FileChange::new(p.clone(), ChangeStatus::ModifiedUntracked)));

    let (mut added, revealed): StampedPaths<'_> = after
        .untracked
        .iter()
        .filter(|(p, _)| !before.untracked.contains_key(*p) && !deleted.contains(*p))
        .partition(|(_, st)| before.taken_ns == 0 || st.mtime_ns >= before.taken_ns - PRE_EXISTING_SLACK_NS);
    // Older than the turn: it was there all along, hidden by .gitignore. Report it like
    // a changed untracked file, so undo refuses rather than deleting the user's file.
    let mut revealed: Vec<&String> = revealed.into_iter().map(|(p, _)| p).collect();
    revealed.sort();
    files.extend(revealed.into_iter().map(|p| FileChange::new(p.clone(), ChangeStatus::ModifiedUntracked)));
    added.sort_by(|a, b| a.0.cmp(b.0));
    files.extend(added.iter().map(|(p, _)| FileChange::new((*p).clone(), ChangeStatus::Added)));
    Merged { files, added: added.into_iter().map(|(p, s)| (p.clone(), *s)).collect() }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot::RepoState;

    #[test]
    fn untracked_changes_merge_into_the_file_list() {
        let st = |size| Stamp { size, mtime_ns: 1 };
        let snap = |tree: &str, files: &[(&str, u64)]| Snapshot { tree: tree.into(), untracked: files.iter().map(|(p, n)| (p.to_string(), st(*n))).collect(), taken_ns: 0, repo: RepoState::default() };
        let fc = |p: &str, status| FileChange::new(p.into(), status);
        let before = snap("t1", &[("keep", 1), ("edit", 1), ("gone", 1), ("staged", 1), ("moved", 1)]);
        let after = snap("t2", &[("keep", 1), ("edit", 2), ("new", 3), ("zz", 1), ("untracked-now", 1)]);
        let tree = vec![
            fc("staged", ChangeStatus::Added),
            fc("untracked-now", ChangeStatus::Deleted),
            fc("tracked", ChangeStatus::Modified),
            FileChange { old_path: Some("old".into()), ..fc("moved", ChangeStatus::Renamed) },
        ];
        // `staged` is on disk unchanged; `gone` and `moved` are not.
        let on_disk = |p: &str| (p == "staged").then(|| st(1));
        let m = merge_untracked(tree, &before, &after, on_disk);
        let got: Vec<_> = m.files.iter().map(|f| (f.path.as_str(), f.status)).collect();
        assert_eq!(
            got,
            [
                ("tracked", ChangeStatus::Modified),
                ("old", ChangeStatus::Deleted),
                ("edit", ChangeStatus::ModifiedUntracked),
                ("gone", ChangeStatus::ModifiedUntracked),
                ("moved", ChangeStatus::ModifiedUntracked),
                ("new", ChangeStatus::Added),
                ("zz", ChangeStatus::Added),
            ]
        );
        assert_eq!(m.added.len(), 2);
    }
}
