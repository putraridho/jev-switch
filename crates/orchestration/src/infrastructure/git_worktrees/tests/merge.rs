//! Applying and undoing a worker's diff on the main tree, and a conflict leaving it alone.

use super::{ run_git, temp_dir, temp_repo };
use crate::application::ports::Worktrees;
use crate::infrastructure::git_worktrees::GitWorktrees;

#[test]
fn real_merge_applies_a_workers_diff_to_the_main_tree() {
    let repo = temp_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    let w = wt.create("w1").unwrap();
    std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
    std::fs::write(w.join("new.txt"), "hello\n").unwrap();
    // The planner has since changed another part of the same file: a 3-way merge keeps both.
    std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
    run_git(&repo, &["add", "f.txt"]);
    wt.apply(&wt.diff(&w).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.join("f.txt")).unwrap(),
        "a\nB-from-worker\nc\nd\nE-from-planner\n"
    );
    assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "hello\n");
}

#[test]
fn real_undo_takes_a_merge_back_out_plain_or_three_way() {
    let repo = temp_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    // A plain merge: the working tree changes, the index does not.
    let w = wt.create("w1").unwrap();
    std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
    std::fs::write(w.join("new.txt"), "hello\n").unwrap();
    let patch = wt.diff(&w).unwrap();
    wt.apply(&patch).unwrap();
    wt.unapply(&patch).unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n");
    assert!(!repo.join("new.txt").exists(), "a file the worker created is gone again");
    assert_eq!(run_git(&repo, &["status", "--porcelain"]), "", "the tree is clean again");
    // A 3-way merge (the planner staged an edit to the same file) stages the result; undo puts the index back too.
    std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
    run_git(&repo, &["add", "f.txt"]);
    let w2 = wt.create("w2").unwrap();
    std::fs::write(w2.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
    let patch = wt.diff(&w2).unwrap();
    wt.apply(&patch).unwrap();
    wt.unapply(&patch).unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.join("f.txt")).unwrap(),
        "a\nb\nc\nd\nE-from-planner\n",
        "the planner's edit stays"
    );
    assert_eq!(
        run_git(&repo, &["status", "--porcelain"]),
        "M  f.txt\n",
        "staged as the planner left it, nothing unstaged"
    );
    // After the files moved on, undo refuses and changes nothing.
    wt.apply(&patch).unwrap();
    std::fs::write(repo.join("f.txt"), "rewritten\n").unwrap();
    assert!(format!("{:#}", wt.unapply(&patch).unwrap_err()).contains("nothing was changed"));
    assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "rewritten\n");
}

#[test]
fn a_conflicting_merge_leaves_the_tree_untouched() {
    let repo = temp_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    let w = wt.create("w1").unwrap();
    std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
    std::fs::write(w.join("new.txt"), "hello\n").unwrap();
    let patch = wt.diff(&w).unwrap();
    // The planner changed the very same line.
    std::fs::write(repo.join("f.txt"), "a\nB-from-planner\nc\nd\ne\n").unwrap();
    run_git(&repo, &["add", "f.txt"]);
    let before_status = run_git(&repo, &["status", "--porcelain"]);
    let before_index = run_git(&repo, &["ls-files", "--stage"]);
    let err = wt.apply(&patch).unwrap_err();
    assert!(format!("{err:#}").contains("nothing was changed"), "{err:#}");
    assert_eq!(
        std::fs::read_to_string(repo.join("f.txt")).unwrap(),
        "a\nB-from-planner\nc\nd\ne\n",
        "no conflict markers"
    );
    assert!(!repo.join("new.txt").exists(), "no partial application");
    assert_eq!(run_git(&repo, &["status", "--porcelain"]), before_status);
    assert_eq!(
        run_git(&repo, &["ls-files", "--stage"]),
        before_index,
        "the index is as it was"
    );
}
