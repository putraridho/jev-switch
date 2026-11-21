//! Putting a patch on the main tree, or taking it back out, without leaving it half changed.

use std::path::PathBuf;

use anyhow::{ anyhow, Result };

use super::runner::git;
use super::snapshot::Snapshot;
use super::GitWorktrees;
use crate::domain::file_patch::patch_paths;

/// Which way a patch goes onto the main tree: a merge, or the undo of one.
#[derive(Debug, Clone, Copy)]
pub(super) enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    pub(super) fn flag(self) -> Option<&'static str> {
        match self {
            Self::Forward => None,
            Self::Reverse => Some("-R"),
        }
    }
}

/// Why a patch did not go on, from git's words for the plain apply and the 3-way one.
fn explain_apply_failure(plain: &str, three_way: &str) -> String {
    let cause = if three_way.contains("does not match index") {
        "your working tree has uncommitted edits to a file the worker also changed, and they overlap its hunks \
         (a 3-way merge also needs those files unmodified or staged). Commit, stage or stash them, or merge by hand"
    } else if plain.contains("already exists") || three_way.contains("already exists") {
        "a file the worker created already exists in your tree"
    } else {
        "the worker's changes do not apply cleanly: they conflict with your tree"
    };
    format!("{cause}; nothing was changed. Plain apply: {plain} 3-way apply: {three_way}")
}

impl GitWorktrees {
    fn index_path(&self) -> Option<PathBuf> {
        let rel = git(&self.repo, &["rev-parse", "--git-path", "index"], None).ok()?;
        Some(self.repo.join(rel.trim()))
    }

    /// A plain `git apply` first: it works on the working tree whatever is unstaged, as
    /// long as the hunks fit. Only when it does not fit, `--3way`, which also merges into
    /// text the planner changed but needs the index to match the files. On failure the
    /// files and index are put back exactly.
    pub(super) fn apply_patch(&self, patch: &str, direction: Direction) -> Result<()> {
        let run = |extra: &[&str]| {
            let args: Vec<&str> = ["apply"]
                .into_iter()
                .chain(direction.flag())
                .chain(extra.iter().copied())
                .chain(["-"])
                .collect();
            git(&self.repo, &args, Some(patch))
        };
        let snapshot = Snapshot::take(&self.repo, self.index_path(), &patch_paths(patch));
        let Err(plain) = run(&["--check"]).and_then(|_| run(&[])) else {
            return Ok(());
        };
        match run(&["--3way"]) {
            Ok(_) => Ok(()),
            Err(three_way) => {
                snapshot.restore();
                Err(
                    anyhow!(
                        "{}",
                        explain_apply_failure(&format!("{plain:#}."), &format!("{three_way:#}"))
                    )
                )
            }
        }
    }
}
