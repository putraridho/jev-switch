//! The guard that keeps worktree removal strictly inside the worktrees root.

use std::path::{ Component, Path, PathBuf };

use anyhow::{ bail, Context, Result };

/// `path` as a canonical location strictly inside `root`, or why it is not one. `path`
/// may be untrusted (it was read from a saved record), so: it must be absolute and free of
/// `..`; its parent is canonicalized (the entry itself may be gone) so a symlinked
/// directory on the way cannot lead outside; the entry may not itself be a symlink; and it
/// may not be `root` itself.
pub(super) fn checked_inside(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("refusing to touch {}: not an absolute path without `..`", path.display());
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("worktrees root {} is not usable", root.display()))?;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        bail!("refusing to touch {}: it has no parent directory and name", path.display());
    };
    let Ok(parent) = parent.canonicalize() else {
        bail!("refusing to touch {}: its parent directory cannot be resolved", path.display());
    };
    let candidate = parent.join(name);
    if candidate == root || !candidate.starts_with(&root) {
        bail!("refusing to touch {}: not strictly inside {}", path.display(), root.display());
    }
    if std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("refusing to touch {}: it is a symlink", path.display());
    }
    Ok(candidate)
}
