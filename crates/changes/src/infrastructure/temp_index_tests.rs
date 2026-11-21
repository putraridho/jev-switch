//! The temporary index: a failed copy, and the fallback that rebuilds it from entries.

use std::path::Path;

use crate::infrastructure::git_working_tree::snapshot;
use crate::test_support::{repo, Scratch};
use crate::infrastructure::git_temp_index::{copy_index_with, skip_worktree_paths};
use crate::infrastructure::git_working_tree::snapshot_with;
use jev_testkit::git_repo::git as run_git;

/// A copy that dies halfway: leaves a truncated file behind, then reports the error.
fn copy_that_fails_halfway(_from: &Path, to: &Path) -> std::io::Result<u64> {
    std::fs::write(to, b"DIRC-truncated")?;
    Err(std::io::Error::other("disk full"))
}

#[test]
fn a_failed_index_copy_leaves_no_partial_file_behind() {
    let s = Scratch::new("copyfail");
    s.write("index", "the real index");
    let tmp = s.path().join("tmp-index");
    assert!(!copy_index_with(&s.path().join("index"), &tmp, copy_that_fails_halfway), "the caller must know the copy failed");
    assert!(!tmp.exists(), "git would read a truncated index as the real one");
}

#[test]
fn a_snapshot_after_a_failed_index_copy_matches_one_after_a_good_copy() {
    let s = repo();
    s.write("a.txt", "edited\n");
    s.write("untracked.txt", "u\n");
    let good = snapshot(s.path()).expect("snapshot");
    let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("a failed copy must not break the snapshot");
    assert_eq!(after_failure.tree, good.tree, "the fallback sees the same tracked files");
    assert_eq!(after_failure.untracked.keys().collect::<Vec<_>>(), good.untracked.keys().collect::<Vec<_>>());
}

#[test]
fn the_fallback_keeps_skip_worktree_entries_of_a_sparse_checkout() {
    let s = repo();
    run_git(s.path(), &["sparse-checkout", "set", "--no-cone", "/a.txt", "/.gitignore", "/src/"]);
    assert!(!s.exists("b.txt"), "b.txt is not checked out");
    let head = run_git(s.path(), &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let good = snapshot(s.path()).expect("snapshot");
    assert_eq!(good.tree, head, "a file that is not checked out is not a deletion");
    let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("the fallback still snapshots");
    assert_eq!(after_failure.tree, head, "the rebuilt index must keep skip-worktree, or `add -u` records b.txt as deleted");
    s.write("a.txt", "edited\n");
    let edited = snapshot_with(s.path(), copy_that_fails_halfway).expect("snapshot");
    assert_eq!(edited.tree, snapshot(s.path()).unwrap().tree, "and a real edit is still seen");
}

#[test]
fn only_paths_tagged_skip_worktree_are_picked() {
    let tagged = b"H a.txt\0S b.txt\0S dir/with space\0h assumed\0s both\0";
    assert_eq!(skip_worktree_paths(tagged), b"b.txt\0dir/with space\0");
    assert!(skip_worktree_paths(b"").is_empty());
}

#[test]
fn the_fallback_keeps_a_skip_worktree_bit_set_by_hand() {
    // No sparse-checkout patterns for `git add` to fall back on: only the bit says b.txt
    // is not checked out (or edited locally on purpose).
    let s = repo();
    run_git(s.path(), &["update-index", "--skip-worktree", "b.txt"]);
    std::fs::remove_file(s.path().join("b.txt")).unwrap();
    s.write("src/c.txt", "local only\n");
    run_git(s.path(), &["update-index", "--skip-worktree", "src/c.txt"]);
    let good = snapshot(s.path()).expect("snapshot");
    assert_eq!(good.tree, run_git(s.path(), &["rev-parse", "HEAD^{tree}"]).trim());
    let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("the fallback still snapshots");
    assert_eq!(after_failure.tree, good.tree, "b.txt is not deleted and src/c.txt not changed in the snapshot");
}

#[test]
fn the_fallback_matches_a_good_copy_for_an_intent_to_add_file() {
    let s = repo();
    s.write("planned.txt", "later\n");
    run_git(s.path(), &["add", "-N", "planned.txt"]);
    let good = snapshot(s.path()).expect("snapshot");
    let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("snapshot");
    assert_eq!(after_failure.tree, good.tree);
}
