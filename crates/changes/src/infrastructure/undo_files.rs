//! The parts of an undo that look at the disk: guards against writing outside the repository
//! or over something that is not ours, and putting files back.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::domain::undo_report::UndoError;
use crate::infrastructure::git_output::git;
use crate::infrastructure::git_temp_index::{remove_temp_index, temp_index_path};

/// True when `target` really lies inside `root_canon`, looking through symlinks: the
/// deepest existing ancestor is resolved, so a directory that is a link pointing out
/// of the repository is refused.
pub(in crate) fn inside_root(root_canon: &Path, target: &Path) -> bool {
    let mut probe = target.parent();
    while let Some(p) = probe {
        if let Ok(real) = std::fs::canonicalize(p) {
            return real.starts_with(root_canon);
        }
        probe = p.parent();
    }
    false
}

pub(in crate) fn io_err(what: &str, path: &str, e: std::io::Error) -> UndoError {
    UndoError::Failed(format!("{what} {path}: {e}"))
}

/// Something that is not a folder where a folder must be (a file or a link in the way of
/// `rel`'s parents) and that this undo is not itself deleting: git would replace it.
pub(in crate) fn blocking_parent(root: &Path, rel: &str, deleting: &HashSet<&str>) -> Option<String> {
    let mut prefix = PathBuf::new();
    let parts: Vec<_> = Path::new(rel).components().collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        prefix.push(part);
        let name = prefix.to_string_lossy();
        if std::fs::symlink_metadata(root.join(&prefix)).is_ok_and(|m| !m.is_dir()) && !deleting.contains(name.as_ref()) {
            return Some(name.into_owned());
        }
    }
    None
}

/// A folder where the file `rel` has to go, holding something the undo does not delete.
pub(in crate) fn folder_in_the_way(root: &Path, rel: &str, deleting: &HashSet<&str>) -> bool {
    let under = format!("{rel}/");
    std::fs::symlink_metadata(root.join(rel)).is_ok_and(|m| m.is_dir()) && !deleting.iter().any(|d| d.starts_with(&under))
}

/// Put `paths` back as they are in `tree`, through git's own checkout so line endings and
/// clean/smudge filters (git-lfs, `.gitattributes` eol rules) apply as they would for the
/// user. A temporary index holds the tree, so the real index is never touched.
pub(in crate) fn restore_paths(root: &Path, tree: &str, paths: &[&str]) -> Result<(), UndoError> {
    if paths.is_empty() {
        return Ok(());
    }
    let tmp = temp_index_path();
    let result = git(root, Some(&tmp), &["read-tree", tree])
        .ok_or_else(|| UndoError::Failed("could not read the old tree".into()))
        .and_then(|_| {
            paths.chunks(200).try_for_each(|chunk| {
                let mut args = vec!["checkout-index", "-f", "--"];
                args.extend(chunk);
                git(root, Some(&tmp), &args).map(drop).ok_or_else(|| UndoError::Failed("git could not put the old files back".into()))
            })
        });
    remove_temp_index(tmp);
    result
}

pub(in crate) fn remove_file_and_empty_parents(root: &Path, rel: &str) -> Result<(), UndoError> {
    let target = root.join(rel);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => std::fs::remove_file(&target).map_err(|e| io_err("could not delete", rel, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("could not look at", rel, e)),
    }
    // Directories the turn created for this file go with it; a non-empty one stays.
    let mut dir = target.parent();
    while let Some(d) = dir.filter(|d| *d != root) {
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{repo, Scratch};

    #[cfg(unix)]
    #[test]
    fn a_directory_that_links_out_of_the_repository_is_not_followed() {
        let s = repo();
        let outside = Scratch::new("outside");
        std::os::unix::fs::symlink(outside.path(), s.path().join("sneaky")).unwrap();
        let root = std::fs::canonicalize(s.path()).unwrap();
        assert!(!inside_root(&root, &s.path().join("sneaky/file.txt")));
        assert!(!inside_root(&root, &s.path().join("sneaky/new/dir/file.txt")), "not-yet-existing parents are resolved from the nearest real one");
        assert!(inside_root(&root, &s.path().join("src/new/file.txt")));
    }
}
