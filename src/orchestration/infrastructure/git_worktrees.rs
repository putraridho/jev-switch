//! Real git worktrees for workers: `git worktree add --detach`, the diff of a worker's
//! changes, and applying or reversing a patch on the main tree without leaving it half
//! changed. The only door to git the delegator has.

use std::path::{ Path, PathBuf };

use anyhow::{ Context, Result };

use crate::orchestration::application::ports::Worktrees;

mod apply;
mod inside;
mod runner;
mod snapshot;
#[cfg(test)]
mod tests;

use apply::Direction;
use inside::checked_inside;
use runner::git;

/// Real worktrees: `git worktree add --detach <root>/<id> HEAD`. Detached, so no branch
/// is created and nothing is left behind once the worktree is removed.
pub struct GitWorktrees {
    /// The top level of the repository: workers start there, and merges apply there.
    repo: PathBuf,
    root: PathBuf,
}

impl GitWorktrees {
    /// `dir` is any directory inside the repository; worktrees go under `root`
    /// (`<state dir>/worktrees`).
    pub fn new(dir: &Path, root: PathBuf) -> Result<Self> {
        let top = git(dir, &["rev-parse", "--show-toplevel"], None).with_context(||
            format!(
                "{} is not inside a git repository, so workers cannot get worktrees",
                dir.display()
            )
        )?;
        Ok(Self { repo: PathBuf::from(top.trim()), root })
    }
}

impl Worktrees for GitWorktrees {
    fn repo(&self) -> PathBuf {
        self.repo.clone()
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn create(&self, id: &str) -> Result<PathBuf> {
        let path = self.root.join(id);
        std::fs
            ::create_dir_all(&self.root)
            .with_context(|| format!("create {}", self.root.display()))?;
        git(&self.repo, &["worktree", "add", "--detach", &path.to_string_lossy(), "HEAD"], None)?;
        Ok(path)
    }

    /// `git add -A` first so files the worker created show up; `--binary` so the patch
    /// applies for binary files too.
    fn diff(&self, path: &Path) -> Result<String> {
        git(path, &["add", "-A"], None)?;
        git(path, &["diff", "--cached", "--binary", "--no-ext-diff", "--no-textconv", "HEAD"], None)
    }

    fn apply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Forward)
    }

    /// The same as a merge, in reverse: a 3-way merge's hunks carry the planner's text as
    /// context, so taking them back out needs the 3-way step too.
    fn unapply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Reverse).context(
            "the merged changes cannot be taken back out cleanly"
        )
    }

    /// `git worktree remove`, and when git will not (the checkout belongs to another
    /// repository, or git's record of it is gone) the directory itself. The path comes from
    /// a saved record, so it is trusted only after [`checked_inside`] proves it is a real
    /// entry strictly inside `root`; nothing else is ever deleted or handed to git.
    fn remove(&self, path: &Path) -> Result<()> {
        let inside = checked_inside(&self.root, path)?;
        match git(&self.repo, &["worktree", "remove", "--force", &path.to_string_lossy()], None) {
            Ok(_) => Ok(()),
            Err(_) if !inside.exists() => Ok(()),
            Err(_) =>
                std::fs
                    ::remove_dir_all(&inside)
                    .with_context(|| format!("remove {}", path.display())),
        }
    }

    fn prune(&self, repo: &Path) -> Result<()> {
        git(repo, &["worktree", "prune"], None).map(|_| ())
    }
}
