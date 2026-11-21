//! Undo in real temporary repositories.

use std::collections::HashMap;
use std::path::Path;

use crate::changes::application::undo_turn::undo;
use crate::changes::domain::change_set::ChangeSummary;
use crate::changes::domain::file_change::{ChangeStatus, FileChange};
use crate::changes::domain::undo_report::UndoError;
use crate::changes::infrastructure::git_tree::GitWorkingTree;
use crate::changes::infrastructure::git_working_tree::{snapshot, summarize};
use crate::changes::test_support::{fake_turn, files_by_path, repo, Scratch};
use crate::testkit::git_repo::git as run_git;

#[test]
fn a_plain_move_is_a_deleted_and_an_added_file_and_undo_moves_it_back() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    std::fs::rename(s.path().join("a.txt"), s.path().join("renamed.txt")).unwrap();
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    let by = files_by_path(&sum);
    assert_eq!(sum.files.len(), 2, "{:?}", sum.files);
    assert_eq!((by["a.txt"].status, by["renamed.txt"].status), (ChangeStatus::Deleted, ChangeStatus::Added));
    assert_eq!(by["renamed.txt"].added, 3);
    undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
    assert!(!s.exists("renamed.txt"));
}

#[test]
fn a_git_mv_is_a_rename_and_undo_moves_it_back() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    run_git(s.path(), &["mv", "a.txt", "renamed.txt"]);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(sum.files.len(), 1, "{:?}", sum.files);
    let f = &sum.files[0];
    assert_eq!((f.status, f.path.as_str(), f.old_path.as_deref()), (ChangeStatus::Renamed, "renamed.txt", Some("a.txt")));
    let report = undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert_eq!(report.removed, ["renamed.txt"]);
    assert_eq!(report.recreated, ["a.txt"]);
    assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
    assert!(!s.exists("renamed.txt"));
}

#[test]
fn undo_restores_modified_added_and_deleted_files_and_leaves_ignored_ones_alone() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("deep/er/new.txt", "x\n");
    fake_turn(&s);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();

    let report = undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert_eq!(report.restored, ["a.txt"]);
    assert_eq!(report.recreated, ["b.txt"]);
    let mut removed = report.removed.clone();
    removed.sort();
    assert_eq!(removed, ["deep/er/new.txt", "new.txt"]);
    assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
    assert_eq!(s.read("b.txt"), "bee\n");
    assert!(!s.exists("new.txt") && !s.exists("deep"), "the folders the turn made go too");
    assert_eq!(s.read("debug.log"), "noise\n", "an ignored file is not ours to touch");
    assert_eq!(snapshot(s.path()).unwrap(), before, "the tree is exactly as before the turn");
    // The repository's own state was never touched.
    assert!(run_git(s.path(), &["status", "--porcelain"]).trim().is_empty());
}

#[cfg(unix)]
#[test]
fn undo_puts_the_executable_bit_and_symlinks_back() {
    use std::os::unix::fs::PermissionsExt;
    let s = repo();
    s.write("run.sh", "#!/bin/sh\n");
    std::fs::set_permissions(s.path().join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("a.txt", s.path().join("link")).unwrap();
    run_git(s.path(), &["add", "-A"]);
    run_git(s.path(), &["commit", "-q", "-m", "more"]);
    let before = snapshot(s.path()).unwrap();
    std::fs::remove_file(s.path().join("run.sh")).unwrap();
    std::fs::remove_file(s.path().join("link")).unwrap();
    s.write("link", "now a plain file\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    undo(&GitWorkingTree, s.path(), &sum).unwrap();
    let mode = std::fs::metadata(s.path().join("run.sh")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
    assert_eq!(std::fs::read_link(s.path().join("link")).unwrap(), Path::new("a.txt"));
    assert_eq!(snapshot(s.path()).unwrap(), before);
}

#[test]
fn undo_refuses_when_a_changed_file_was_edited_again_and_touches_nothing() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    fake_turn(&s);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();

    s.write("new.txt", "x\ny\nz\nand the user added this\n");
    s.write("unrelated.txt", "not part of the turn\n");
    let err = undo(&GitWorkingTree, s.path(), &sum).unwrap_err();
    assert_eq!(err, UndoError::ChangedSince(vec!["new.txt".into()]), "only the edited file is named, not unrelated ones");
    assert!(err.to_string().contains("new.txt"));
    assert_eq!(s.read("a.txt"), "one\nTWO\nthree\nfour\n", "nothing was undone");
    assert!(!s.exists("b.txt"));

    // A deleted file that came back, and a modified one edited again, are caught too.
    // (new.txt is untracked, so rewriting it, even with the same text, changes its
    // mtime and counts as an edit: we cannot compare content we never hashed.)
    s.write("new.txt", "x\ny\nz\n");
    s.write("b.txt", "back again\n");
    s.write("a.txt", "edited again\n");
    match undo(&GitWorkingTree, s.path(), &sum).unwrap_err() {
        UndoError::ChangedSince(mut files) => {
            files.sort();
            assert_eq!(files, ["a.txt", "b.txt", "new.txt"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn undo_works_from_a_subdirectory_of_the_repository() {
    let s = repo();
    let sub = s.path().join("src");
    let before = snapshot(&sub).unwrap();
    fake_turn(&s);
    let after = snapshot(&sub).unwrap();
    let sum = summarize(&sub, &before, &after).unwrap();
    assert!(files_by_path(&sum).contains_key("a.txt"), "paths are relative to the repository root");
    undo(&GitWorkingTree, &sub, &sum).unwrap();
    assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
}

#[test]
fn undo_refuses_a_summary_with_a_path_outside_the_repository() {
    let s = repo();
    let t = snapshot(s.path()).unwrap();
    let evil = ChangeSummary {
        files: vec![FileChange { path: "../escape.txt".into(), status: ChangeStatus::Added, added: 1, removed: 0, old_path: None }],
        before: t.tree.clone(),
        after: t.tree,
        added_stamps: HashMap::new(),
        ..ChangeSummary::default()
    };
    std::fs::write(s.path().parent().unwrap().join("escape.txt"), "mine").ok();
    assert!(matches!(undo(&GitWorkingTree, s.path(), &evil), Err(UndoError::UnsafePath(_))));
    assert_eq!(std::fs::read_to_string(s.path().parent().unwrap().join("escape.txt")).unwrap(), "mine");
    let _ = std::fs::remove_file(s.path().parent().unwrap().join("escape.txt"));
}

/// Write `text` and give the file a fixed mtime, so two writes never look alike by luck.
/// Write `rel` with its mtime `secs` after `base`. Each test takes its own `base` when it
/// starts, just before its snapshots: deterministic stamps that still count as "made
/// during the turn" (a base shared by the whole test process drifts past the 2 s slack
/// once the suite has been running a while).
fn write_at(s: &Scratch, base: std::time::SystemTime, rel: &str, text: &str, secs: u64) {
    s.write(rel, text);
    let f = std::fs::OpenOptions::new().write(true).open(s.path().join(rel)).unwrap();
    f.set_modified(base + std::time::Duration::from_secs(secs)).unwrap();
}

#[test]
fn a_changed_untracked_file_is_reported_and_undo_refuses_to_touch_anything() {
    let s = repo();
    let t0 = std::time::SystemTime::now();
    write_at(&s, t0, "notes.txt", "mine\n", 0);
    write_at(&s, t0, "same.txt", "stays\n", 0);
    write_at(&s, t0, "gone.txt", "bye\n", 0);
    let before = snapshot(s.path()).unwrap();
    // Same size, different mtime still counts; so does a delete.
    write_at(&s, t0, "notes.txt", "MINE\n", 5);
    std::fs::remove_file(s.path().join("gone.txt")).unwrap();
    s.write("a.txt", "one\nTWO\nthree\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    let by = files_by_path(&sum);
    assert_eq!(sum.files.len(), 3, "an untouched untracked file is not a change: {:?}", sum.files);
    assert_eq!(by["a.txt"].status, ChangeStatus::Modified);
    assert_eq!(by["notes.txt"].status, ChangeStatus::ModifiedUntracked);
    assert_eq!(by["gone.txt"].status, ChangeStatus::ModifiedUntracked);

    let err = undo(&GitWorkingTree, s.path(), &sum).unwrap_err();
    assert_eq!(err, UndoError::UntrackedModified(vec!["gone.txt".into(), "notes.txt".into()]));
    let msg = err.to_string();
    assert!(msg.contains("notes.txt") && msg.contains("gone.txt") && msg.contains("no earlier copy"), "{msg}");
    assert_eq!(s.read("notes.txt"), "MINE\n");
    assert_eq!(s.read("a.txt"), "one\nTWO\nthree\n", "nothing else was undone either");
}

#[test]
fn an_untracked_file_the_turn_added_is_deleted_by_undo_only_while_it_is_as_the_turn_left_it() {
    let s = repo();
    let t0 = std::time::SystemTime::now();
    let before = snapshot(s.path()).unwrap();
    write_at(&s, t0, "new.txt", "made by the turn\n", 0);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(sum.files.len(), 1);
    assert_eq!((sum.files[0].status, sum.files[0].added), (ChangeStatus::Added, 1));
    assert!(sum.added_stamps.contains_key("new.txt"));

    write_at(&s, t0, "new.txt", "made by the turn\n", 9); // touched by someone since
    assert_eq!(undo(&GitWorkingTree, s.path(), &sum), Err(UndoError::ChangedSince(vec!["new.txt".into()])));
    assert!(s.exists("new.txt"));
    write_at(&s, t0, "new.txt", "made by the turn\n", 0);
    assert_eq!(undo(&GitWorkingTree, s.path(), &sum).unwrap().removed, ["new.txt"]);
    assert!(!s.exists("new.txt"));
}

#[test]
fn an_untracked_file_the_user_recreated_at_a_deleted_path_is_not_overwritten() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    run_git(s.path(), &["rm", "-q", "b.txt"]); // the turn deletes it from the index too
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    s.write("b.txt", "the user wrote this meanwhile\n"); // untracked now
    assert_eq!(undo(&GitWorkingTree, s.path(), &sum), Err(UndoError::ChangedSince(vec!["b.txt".into()])));
    assert_eq!(s.read("b.txt"), "the user wrote this meanwhile\n");
}

#[test]
fn a_pre_existing_untracked_file_the_turn_only_staged_is_not_an_addition_to_delete() {
    let s = repo();
    let t0 = std::time::SystemTime::now();
    write_at(&s, t0, "notes.txt", "mine\n", 0);
    let before = snapshot(s.path()).unwrap();
    run_git(s.path(), &["add", "notes.txt"]);
    s.write("a.txt", "one\nTWO\nthree\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(sum.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert_eq!(s.read("notes.txt"), "mine\n", "it was there before the turn");
}

/// Commit `rel` (written as `text`), run a turn that changes it, undo; give the bytes after.
fn commit_change_undo(s: &Scratch, rel: &str, text: &str, changed: &str) -> Vec<u8> {
    std::fs::write(s.path().join(rel), text).unwrap();
    run_git(s.path(), &["add", "-A"]);
    run_git(s.path(), &["commit", "-q", "-m", "add"]);
    let before = snapshot(s.path()).unwrap();
    std::fs::write(s.path().join(rel), changed).unwrap();
    let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
    undo(&GitWorkingTree, s.path(), &sum).unwrap_or_else(|e| panic!("{e}"));
    std::fs::read(s.path().join(rel)).unwrap()
}

#[test]
fn undo_gives_back_crlf_files_under_autocrlf() {
    let s = repo();
    run_git(s.path(), &["config", "core.autocrlf", "true"]);
    let original = "line one\r\nline two\r\n";
    let restored = commit_change_undo(&s, "win.txt", original, "line one\r\nCHANGED\r\n");
    assert_eq!(restored, original.as_bytes(), "byte-identical to the file the user had");
    assert!(run_git(s.path(), &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn undo_applies_gitattributes_eol_rules() {
    let s = repo();
    s.write(".gitattributes", "win.dat text eol=crlf\n");
    let original = "one\r\ntwo\r\n";
    let restored = commit_change_undo(&s, "win.dat", original, "one\r\nCHANGED\r\n");
    assert_eq!(restored, original.as_bytes());
}

#[test]
fn undo_runs_the_smudge_filter_so_filtered_files_come_back_whole() {
    let s = repo();
    // The same code path git-lfs takes: the stored blob is the clean form.
    run_git(s.path(), &["config", "filter.up.clean", "sed s/secret/XXX/"]);
    run_git(s.path(), &["config", "filter.up.smudge", "sed s/XXX/secret/"]);
    s.write(".gitattributes", "f.dat filter=up\n");
    let original = "my secret value\n";
    let restored = commit_change_undo(&s, "f.dat", original, "something else\n");
    assert_eq!(String::from_utf8(restored).unwrap(), original, "not the clean form (`XXX`)");
    assert_eq!(run_git(s.path(), &["show", "HEAD:f.dat"]), "my XXX value\n", "the repository itself holds the clean form");
}

#[test]
fn restoring_leaves_the_real_index_alone() {
    let s = repo();
    run_git(s.path(), &["config", "core.autocrlf", "true"]);
    std::fs::write(s.path().join("w.txt"), "x\r\ny\r\n").unwrap();
    run_git(s.path(), &["add", "-A"]);
    run_git(s.path(), &["commit", "-q", "-m", "w"]);
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "changed\n");
    std::fs::write(s.path().join("w.txt"), "z\r\n").unwrap();
    let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
    run_git(s.path(), &["add", "a.txt"]); // the user stages work after the turn
    let index_before = std::fs::read(s.path().join(".git/index")).unwrap();
    // a.txt is staged now, but its working copy is still as the turn left it.
    undo(&GitWorkingTree, s.path(), &sum).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(std::fs::read(s.path().join(".git/index")).unwrap(), index_before);
    assert_eq!(std::fs::read(s.path().join("w.txt")).unwrap(), b"x\r\ny\r\n");
}
