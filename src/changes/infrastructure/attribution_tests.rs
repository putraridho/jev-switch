//! What a turn is credited with when other processes change files meanwhile.


use crate::changes::application::undo_turn::undo;
use crate::changes::domain::turn_activity::TurnActivity;
use crate::changes::domain::undo_report::UndoError;
use crate::changes::infrastructure::git_working_tree::{diff_text, snapshot, summarize, summarize_turn};
use crate::changes::test_support::{repo, tools};
use crate::testkit::git_repo::git as run_git;

#[test]
fn a_branch_switched_by_someone_else_during_a_read_only_turn_is_not_the_turns() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    // another process: new branch with a commit that touches several files, checked out
    run_git(s.path(), &["checkout", "-q", "-b", "other"]);
    s.write("a.txt", "merged\n");
    s.write("docs/new.md", "doc\n");
    run_git(s.path(), &["add", "-A"]);
    run_git(s.path(), &["commit", "-q", "-m", "other"]);
    let after = snapshot(s.path()).unwrap();
    let raw = summarize(s.path(), &before, &after).unwrap();
    assert_eq!(raw.files.len(), 2, "the plain diff blames the turn for both");

    let sum = summarize_turn(s.path(), &before, &after, &tools(&["Grep foo"])).unwrap();
    assert!(sum.files.is_empty(), "{:?}", sum.files);
    assert_eq!(sum.outside.files.len(), 2);
    let moved = sum.outside.moved.as_ref().expect("the move is reported");
    assert!(moved.describe().contains("→ other"), "{}", moved.describe());
    let warning = sum.outside.warning().unwrap();
    assert!(warning.contains("2 files changed outside this turn") && warning.contains("not included"), "{warning}");
    assert!(!sum.is_empty(), "the warning must survive to the end of the turn");
    assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
    assert_eq!(s.read("a.txt"), "merged\n", "nothing was reverted");
    assert_eq!(diff_text(s.path(), &sum, None).as_deref(), Some(""), "/diff shows none of it");
}

#[test]
fn a_moved_head_refuses_undo_even_for_files_the_turn_wrote() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "mine\n");
    run_git(s.path(), &["commit", "-q", "-am", "someone commits"]);
    let after = snapshot(s.path()).unwrap();
    let abs = s.path().join("a.txt");
    let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display())])).unwrap();
    assert_eq!(sum.files.len(), 1, "the edit is still shown");
    assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
    assert_eq!(s.read("a.txt"), "mine\n");
}

#[test]
fn a_doc_written_by_someone_else_during_a_read_only_turn_is_outside() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("docs/notes.md", "from another session\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize_turn(s.path(), &before, &after, &tools(&["Grep todo", "Read a.txt"])).unwrap();
    assert!(sum.files.is_empty());
    assert_eq!(sum.outside.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["docs/notes.md"]);
    assert!(sum.outside.moved.is_none() && !sum.outside.uncertain);
    assert!(sum.outside.warning().unwrap().contains("1 file changed outside this turn"));
    assert!(matches!(undo(s.path(), &sum), Err(UndoError::NothingToUndo)));
    assert_eq!(s.read("docs/notes.md"), "from another session\n");
}

#[test]
fn only_the_files_the_edit_tools_named_are_attributed_and_undone() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "one\nTWO\nthree\n");
    s.write("theirs.txt", "not mine\n");
    std::fs::write(s.path().join("b.txt"), "someone else edits b\n").unwrap();
    let after = snapshot(s.path()).unwrap();
    let abs = s.path().join("a.txt");
    let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display()), "Read b.txt"])).unwrap();
    assert_eq!(sum.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    let mut outside: Vec<&str> = sum.outside.files.iter().map(|f| f.path.as_str()).collect();
    outside.sort();
    assert_eq!(outside, ["b.txt", "theirs.txt"]);
    let diff = diff_text(s.path(), &sum, None).unwrap();
    assert!(diff.contains("a.txt") && !diff.contains("b.txt"), "{diff}");
    undo(s.path(), &sum).unwrap();
    assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
    assert_eq!(s.read("b.txt"), "someone else edits b\n");
    assert_eq!(s.read("theirs.txt"), "not mine\n");
    // a relative path from the turn's own folder matches the same way
    let rel = summarize_turn(s.path(), &before, &after, &tools(&["Edit a.txt"])).unwrap();
    assert_eq!(rel.files.len(), 1);
}

#[test]
fn a_turn_that_reported_no_tools_keeps_its_changes_marked_uncertain() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "edited without a tool line\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize_turn(s.path(), &before, &after, &TurnActivity::default()).unwrap();
    assert_eq!(sum.files.len(), 1, "unreported edits are not hidden");
    assert!(sum.outside.files.is_empty() && sum.outside.uncertain);
}

#[test]
fn after_a_shell_command_every_change_is_kept_but_marked_uncertain() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("generated.txt", "by a script\n");
    let after = snapshot(s.path()).unwrap();
    let sum = summarize_turn(s.path(), &before, &after, &tools(&["Bash ./gen.sh"])).unwrap();
    assert_eq!(sum.files.len(), 1);
    assert!(sum.outside.files.is_empty() && sum.outside.uncertain);
    assert!(sum.outside.warning().unwrap().contains("shell command ran"));
    // every file named by an edit tool: nothing to doubt
    let abs = s.path().join("generated.txt");
    let named = summarize_turn(s.path(), &before, &after, &tools(&["Bash ls", &format!("Write {}", abs.display())])).unwrap();
    assert!(!named.outside.uncertain);
}

#[test]
fn undo_refuses_when_the_branch_changed_after_the_turn() {
    let s = repo();
    let before = snapshot(s.path()).unwrap();
    s.write("a.txt", "one\nTWO\nthree\n");
    let after = snapshot(s.path()).unwrap();
    let abs = s.path().join("a.txt");
    let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display())])).unwrap();
    run_git(s.path(), &["checkout", "-q", "-b", "later"]);
    assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
    assert_eq!(s.read("a.txt"), "one\nTWO\nthree\n");
    run_git(s.path(), &["checkout", "-q", "-"]);
    undo(s.path(), &sum).unwrap();
}
