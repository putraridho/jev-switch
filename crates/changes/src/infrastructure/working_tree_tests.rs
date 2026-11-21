//! Snapshot, summary and diff in real temporary repositories.

use std::collections::HashMap;
use std::path::Path;

use crate::application::undo_turn::undo;
use crate::infrastructure::git_tree::GitWorkingTree;
use crate::domain::change_set::ChangeSummary;
use crate::domain::file_change::{ChangeStatus, FileChange};
use crate::domain::undo_report::UndoError;
use crate::infrastructure::git_working_tree::{diff_text, snapshot, summarize};
use crate::test_support::{fake_turn, files_by_path, repo, Scratch};
use crate::domain::snapshot::{RepoState, Snapshot};
use jev_testkit::git_repo::git as run_git;

#[test]
fn a_snapshot_lists_untracked_files_skips_ignored_ones_and_touches_nothing() {
    let s = repo();
    let status_before = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
    let index_before = std::fs::read(s.path().join(".git/index")).unwrap();
    let t1 = snapshot(s.path()).expect("a tree id");
    assert!(t1.untracked.is_empty());
    s.write("untracked.txt", "u\n");
    s.write("ignored.log", "i\n");
    s.write("deep/er/more.txt", "m\n");
    let t2 = snapshot(s.path()).unwrap();
    assert_eq!(t1.tree, t2.tree, "untracked files are not in the tracked tree");
    let mut names: Vec<_> = t2.untracked.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["deep/er/more.txt", "untracked.txt"], "ignored files are left out, paths are repository-relative");
    assert_eq!(t2.untracked["untracked.txt"].size, 2);
    assert!(!run_git(s.path(), &["ls-tree", "-r", "--name-only", &t2.tree]).contains("untracked.txt"));
    // The user's index is as it was and the new file is still untracked, not staged.
    assert_eq!(std::fs::read(s.path().join(".git/index")).unwrap(), index_before);
    let status = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
    assert!(status.contains("?? untracked.txt"), "{status}");
    assert_ne!(status, status_before);
    // Nothing changed, nothing differs.
    assert_eq!(snapshot(s.path()).unwrap(), t2);
    // A tracked edit moves the tree.
    s.write("a.txt", "edited\n");
    assert_ne!(snapshot(s.path()).unwrap().tree, t1.tree);
}

#[test]
fn the_summary_lists_added_modified_and_deleted_files_with_line_counts() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    fake_turn(&s);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!((sum.before.as_str(), sum.after.as_str()), (before.tree.as_str(), after.tree.as_str()));
    let by = files_by_path(&sum);
    assert_eq!(sum.files.len(), 3, "{:?}", sum.files);
    assert_eq!((by["a.txt"].status, by["a.txt"].added, by["a.txt"].removed), (ChangeStatus::Modified, 2, 1));
    assert_eq!((by["new.txt"].status, by["new.txt"].added, by["new.txt"].removed), (ChangeStatus::Added, 3, 0));
    assert_eq!((by["b.txt"].status, by["b.txt"].added, by["b.txt"].removed), (ChangeStatus::Deleted, 0, 1));
    assert!(!by.contains_key("debug.log"), "ignored files are not changes");
    assert_eq!(sum.totals(), (5, 2));

    let diff = diff_text(s.path(), &sum, None).unwrap();
    assert!(diff.contains("+TWO") && diff.contains("-two") && diff.contains("+++ b/new.txt") && diff.contains("--- a/b.txt"), "{diff}");
    assert!(!diff.contains('\u{1b}'), "no colour codes");
    let one = diff_text(s.path(), &sum, Some("new.txt")).unwrap();
    assert!(one.contains("new.txt") && !one.contains("a.txt"), "{one}");
    // A glob is a name here, not a pattern.
    assert_eq!(diff_text(s.path(), &sum, Some("*.txt")).unwrap(), "");
}

#[test]
fn nothing_changed_gives_an_empty_summary() {
    let s = repo();
    let t = snapshot(s.path()).unwrap();
    assert!(summarize(s.path(), &t, &snapshot(s.path()).unwrap()).unwrap().is_empty());
}

#[test]
fn outside_a_repository_everything_degrades_to_none() {
    let s = Scratch::new("plain");
    s.write("f.txt", "x");
    assert_eq!(snapshot(s.path()), None);
    let t = "0".repeat(40);
    let snap = |tree: String| Snapshot { tree, untracked: HashMap::new(), taken_ns: 0, repo: RepoState::default() };
    assert_eq!(summarize(s.path(), &snap(t.clone()), &snap("1".repeat(40))), None);
    let sum = ChangeSummary {
        files: vec![FileChange { path: "f.txt".into(), status: ChangeStatus::Added, added: 1, removed: 0, old_path: None }],
        before: t.clone(),
        after: t,
        added_stamps: HashMap::new(),
        ..ChangeSummary::default()
    };
    assert_eq!(diff_text(s.path(), &sum, None), None);
    assert!(matches!(undo(&GitWorkingTree, s.path(), &sum), Err(UndoError::Failed(_))));
    assert_eq!(snapshot(Path::new("/definitely/not/a/dir")), None);
}

#[test]
fn a_fresh_repository_without_commits_can_be_snapshotted() {
    let s = Scratch::new("fresh");
    run_git(s.path(), &["init", "-q"]);
    let before = snapshot(s.path()).expect("empty tree");
    s.write("first.txt", "hi\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(sum.files.len(), 1);
    undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert!(!s.exists("first.txt"));
}

/// `git hash-object` WITHOUT `-w`: the id a blob with this content would have.
/// The blob id git would give `content`, computed WITHOUT writing it (`hash-object`
/// with no `-w`) from a temp file outside the repo, via the shared `run_git` helper,
/// which fails with git's own message if anything goes wrong.
fn blob_id(dir: &Path, content: &str) -> String {
    let tmp = std::env::temp_dir().join(format!("jev-blob-{}-{:x}", std::process::id(), content.len() as u64 ^ 0x9e37_79b9));
    std::fs::write(&tmp, content).unwrap();
    let id = run_git(dir, &["hash-object", "--no-filters", tmp.to_str().unwrap()]).trim().to_string();
    let _ = std::fs::remove_file(&tmp);
    id
}

fn object_exists(dir: &Path, id: &str) -> bool {
    run_git(dir, &["cat-file", "--batch-all-objects", "--batch-check"]).lines().any(|l| l.starts_with(id))
}

#[test]
fn untracked_files_are_never_written_into_the_object_store() {
    let s = repo();
    let secret = "API_KEY=unique-secret-7f3a9c1e-not-in-git\n";
    let id = blob_id(s.path(), secret);
    assert!(!object_exists(s.path(), &id), "the check itself: the blob is not there yet");
    s.write("secret.env", secret);
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "one\nTWO\nthree\n");
    s.write("data/big.bin", "dataset\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(sum.files.len(), 2, "{:?}", sum.files);
    diff_text(s.path(), &sum, None).unwrap();
    undo(&GitWorkingTree, s.path(), &sum).unwrap();
    assert!(!object_exists(s.path(), &id), "the untracked secret must not be hashed into .git/objects");
    assert!(!object_exists(s.path(), &blob_id(s.path(), "dataset\n")));
    assert!(s.exists("secret.env"), "it was there before the turn, so undo leaves it");
}

#[test]
fn a_tracked_file_the_turn_only_removed_from_the_index_is_still_there_and_not_a_change() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    run_git(s.path(), &["rm", "-q", "--cached", "b.txt"]);
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    assert!(sum.is_empty(), "{:?}", sum.files);
    assert!(s.exists("b.txt"));
}

#[test]
fn the_diff_shows_new_untracked_files_without_git_hashing_them() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("new.txt", "x\ny\nz\n");
    s.write("a.txt", "one\nTWO\nthree\n");
    let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
    let all = diff_text(s.path(), &sum, None).unwrap();
    assert!(all.contains("+TWO") && all.contains("+++ b/new.txt") && all.contains("+y\n"), "{all}");
    let one = diff_text(s.path(), &sum, Some("new.txt")).unwrap();
    assert!(one.contains("+++ b/new.txt") && !one.contains("a.txt"), "{one}");
    let other = diff_text(s.path(), &sum, Some("a.txt")).unwrap();
    assert!(other.contains("+TWO") && !other.contains("new.txt"), "{other}");
    assert!(!object_exists(s.path(), &blob_id(s.path(), "x\ny\nz\n")));
}

#[test]
fn diff_of_one_file_works_when_the_project_is_a_subfolder_of_the_repository() {
    let s = repo();
    let sub = s.path().join("src");
    let before = snapshot(&sub).unwrap();
    fake_turn(&s);
    s.write("src/c.txt", "see\nmore\n");
    let sum = summarize(&sub, &before, &snapshot(&sub).unwrap()).unwrap();
    // The summary names files from the repository root, and so does the diff.
    for path in ["a.txt", "src/c.txt", "new.txt"] {
        assert!(sum.files.iter().any(|f| f.path == path), "{path} in {:?}", sum.files);
        let d = diff_text(&sub, &sum, Some(path)).unwrap();
        assert!(d.contains(&format!("+++ b/{path}")), "{path}: {d}");
    }
    assert!(diff_text(&sub, &sum, Some("a.txt")).unwrap().contains("+TWO"));
}

#[test]
fn a_hidden_file_revealed_by_a_gitignore_edit_is_never_deleted_by_undo() {
    let s = repo();
    // An ignored file the user had long before the turn (say a secret).
    s.write("secret.log", "TOKEN=abc\n");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    std::fs::File::options().write(true).open(s.path().join("secret.log")).unwrap().set_modified(old).unwrap();
    let before = snapshot(s.path()).unwrap();
    // The turn un-ignores it by rewriting .gitignore, and adds a genuinely new file.
    s.write(".gitignore", "# nothing ignored now\n");
    s.write("made.txt", "new\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize(s.path(), &before, &after).unwrap();
    let by = files_by_path(&sum);
    assert_eq!(by["secret.log"].status, ChangeStatus::ModifiedUntracked, "it existed before: not 'added'");
    assert!(!sum.added_stamps.contains_key("secret.log"));
    assert_eq!(by["made.txt"].status, ChangeStatus::Added);
    // Undo refuses as a whole rather than deleting the user's file.
    assert!(matches!(undo(&GitWorkingTree, s.path(), &sum), Err(UndoError::UntrackedModified(_))));
    assert_eq!(std::fs::read_to_string(s.path().join("secret.log")).unwrap(), "TOKEN=abc\n");
}

#[test]
fn a_same_size_edit_in_the_same_second_as_the_commit_is_never_missed() {
    std::thread::scope(|scope| {
        for worker in 0..4 {
            scope.spawn(move || {
                for _ in (worker..30).step_by(4) {
                    let s = repo();
                    let before = snapshot(s.path()).unwrap();
                    s.write("a.txt", "ONE\ntwo\nthree\n");
                    let after = snapshot(s.path()).unwrap();
                    assert_ne!(before.tree, after.tree, "the edit must be seen");
                    let sum = summarize(s.path(), &before, &after).unwrap();
                    assert_eq!(sum.files.len(), 1);
                    undo(&GitWorkingTree, s.path(), &sum).unwrap();
                    assert_eq!(std::fs::read_to_string(s.path().join("a.txt")).unwrap(), "one\ntwo\nthree\n");
                }
            });
        }
    });
}
