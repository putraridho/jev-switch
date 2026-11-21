//! Creating, diffing and removing real worktrees.

use super::{ run_git, temp_dir, temp_repo };
use crate::application::ports::Worktrees;
use crate::domain::file_patch::patch_paths;
use crate::infrastructure::git_worktrees::GitWorktrees;

#[test]
fn real_worktrees_are_isolated_and_diff_includes_new_files() {
    let repo = temp_repo();
    let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
    let (a, b) = (wt.create("wa").unwrap(), wt.create("wb").unwrap());
    assert_ne!(a, b);
    std::fs::write(a.join("f.txt"), "a\nB-from-a\nc\nd\ne\n").unwrap();
    std::fs::write(b.join("new.txt"), "hello\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(b.join("f.txt")).unwrap(),
        "a\nb\nc\nd\ne\n",
        "b does not see a's edit"
    );
    assert!(!a.join("new.txt").exists(), "a does not see b's file");
    assert_eq!(
        std::fs::read_to_string(repo.join("f.txt")).unwrap(),
        "a\nb\nc\nd\ne\n",
        "the main tree is untouched"
    );
    let (da, db) = (wt.diff(&a).unwrap(), wt.diff(&b).unwrap());
    assert_eq!(patch_paths(&da), ["f.txt"]);
    assert_eq!(patch_paths(&db), ["new.txt"]);
    wt.remove(&a).unwrap();
    assert!(!a.exists() && b.exists());
}

#[test]
fn worktrees_need_a_git_repository() {
    let not_a_repo = temp_dir("plain");
    let err = GitWorktrees::new(&not_a_repo, temp_dir("root")).err().expect("not a repository");
    assert!(format!("{err:#}").contains("not inside a git repository"));
}

#[test]
fn a_real_checkout_that_is_gone_is_cleaned_by_remove_and_prune() {
    let repo = temp_repo();
    let root = temp_dir("wt-root");
    let wt = GitWorktrees::new(&repo, root.clone()).unwrap();
    let (gone, kept) = (wt.create("wgone").unwrap(), wt.create("wkept").unwrap());
    std::fs::remove_dir_all(&gone).unwrap();
    assert!(
        run_git(&repo, &["worktree", "list"]).contains("wgone"),
        "git still lists the vanished checkout"
    );
    wt.prune(&repo).unwrap();
    let list = run_git(&repo, &["worktree", "list"]);
    assert!(!list.contains("wgone") && list.contains("wkept"), "{list}");
    // A directory git does not know (a leftover of another repository) is deleted, but only under the root.
    let stray = root.join("wstray");
    std::fs::create_dir_all(&stray).unwrap();
    wt.remove(&stray).unwrap();
    assert!(!stray.exists());
    let outside = temp_dir("outside");
    assert!(wt.remove(&outside).is_err() && outside.exists(), "never deletes outside its root");
    wt.remove(&kept).unwrap();
}
