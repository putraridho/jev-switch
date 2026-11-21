//! Merging onto a main tree the planner has edited but not staged.

use super::{ numbered_repo, replace_line, run_git, temp_dir };
use crate::application::ports::Worktrees;
use crate::infrastructure::git_worktrees::GitWorktrees;

#[test]
fn merge_works_on_top_of_the_planners_unstaged_edit_to_the_same_file() {
    let repo = numbered_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    let w = wt.create("w1").unwrap();
    replace_line(&w.join("big.txt"), 3, "from-worker");
    let patch = wt.diff(&w).unwrap();
    // The planner edits another part of the same file and has NOT staged it.
    replace_line(&repo.join("big.txt"), 27, "from-planner");
    wt.apply(&patch).unwrap();
    let merged = std::fs::read_to_string(repo.join("big.txt")).unwrap();
    assert!(merged.contains("from-worker") && merged.contains("from-planner"), "{merged}");
    assert!(
        !merged.contains("line3\n") &&
            !merged.contains("line27\n") &&
            !merged.contains("<<<<<<<")
    );
    assert_eq!(
        run_git(&repo, &["diff", "--cached", "--stat"]).trim(),
        "",
        "the planner's index is not touched"
    );
}

#[test]
fn an_overlapping_unstaged_edit_fails_cleanly_and_says_why() {
    let repo = numbered_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    let w = wt.create("w1").unwrap();
    replace_line(&w.join("big.txt"), 3, "from-worker");
    std::fs::write(w.join("new.txt"), "hello\n").unwrap();
    let patch = wt.diff(&w).unwrap();
    replace_line(&repo.join("big.txt"), 4, "from-planner"); // inside the worker's hunk, unstaged
    let (status, before) = (
        run_git(&repo, &["status", "--porcelain"]),
        std::fs::read_to_string(repo.join("big.txt")).unwrap(),
    );
    let err = format!("{:#}", wt.apply(&patch).unwrap_err());
    assert!(err.contains("uncommitted edits") && err.contains("nothing was changed"), "{err}");
    assert!(
        !err.contains("conflict with your tree"),
        "the cause is the dirty file, not a conflict: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("big.txt")).unwrap(),
        before,
        "the file is exactly as it was"
    );
    assert!(!repo.join("new.txt").exists(), "no partial application");
    assert_eq!(run_git(&repo, &["status", "--porcelain"]), status);
}
