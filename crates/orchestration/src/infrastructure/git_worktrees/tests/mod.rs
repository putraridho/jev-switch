//! Tests for the git worktree adapter; the shared helpers make real git repositories.

use std::path::{ Path, PathBuf };

use super::runner::git;

mod dirty_tree;
mod merge;
mod worktrees;


pub(super) fn temp_dir(tag: &str) -> PathBuf {
    jev_testkit::scratch_dir::unique_scratch_dir("delegate", tag)
}

pub(super) fn run_git(dir: &Path, args: &[&str]) -> String {
    git(dir, args, None).unwrap_or_else(|e| panic!("{e:#}"))
}

/// A repository with one commit holding `f.txt` (five lines).
pub(super) fn temp_repo() -> PathBuf {
    let d = temp_dir("repo");
    jev_testkit::git_repo::init_with_commit(&d, &[("f.txt", "a\nb\nc\nd\ne\n")]);
    d
}

/// A repository with `big.txt` of 30 numbered lines, committed.
pub(super) fn numbered_repo() -> PathBuf {
    let d = temp_dir("repo");
    jev_testkit::git_repo::init_with_commit(
        &d,
        &[("big.txt", &(1..=30).map(|n| format!("line{n}\n")).collect::<String>())]
    );
    d
}

pub(super) fn replace_line(path: &Path, n: usize, text: &str) {
    let content = std::fs::read_to_string(path).unwrap();
    let out: String = content
        .lines()
        .enumerate()
        .map(|(i, l)| format!("{}\n", if i + 1 == n { text } else { l }))
        .collect();
    std::fs::write(path, out).unwrap();
}
