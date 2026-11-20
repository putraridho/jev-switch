//! What a turn changed in the working tree, and `/diff` and `/undo` of it.

use super::*;

/// A fake turn that edits a.txt, adds new.txt, deletes b.txt and writes an ignored file.
const EDITS: &str = "echo three >> a.txt; echo fresh > new.txt; rm b.txt; echo noise > debug.log; printf 'FIN:edited\\n'";

fn tracked(repo: &Repo, ready: &[ProviderId]) -> Rig {
    let mut r = rig_in(repo.0.clone(), &states_for(ready), OK);
    r.engine = r.engine.with_track_changes(true);
    r
}

#[tokio::test]
async fn a_turn_reports_what_it_changed_and_the_engine_can_diff_and_undo_it() {
    let repo = Repo::new("undo", true);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, EDITS);
    let report = r.turn("edit things", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);

    let sum = report.changes.clone().expect("a git repo");
    let mut paths: Vec<_> = sum.files.iter().map(|f| (f.path.as_str(), f.status)).collect();
    paths.sort_by_key(|p| p.0);
    use crate::changes::infrastructure::git_working_tree::ChangeStatus::{Added, Deleted, Modified};
    assert_eq!(paths, [("a.txt", Modified), ("b.txt", Deleted), ("new.txt", Added)], "ignored files are not changes");
    assert_eq!(r.engine.last_changes(), Some(&sum));
    let diff = r.engine.last_diff(None).unwrap();
    assert!(diff.contains("+three") && diff.contains("+++ b/new.txt"), "{diff}");
    assert!(!r.engine.last_diff(Some("new.txt")).unwrap().contains("a.txt"));

    let undone = r.engine.undo_last().unwrap();
    assert_eq!(undone.restored, ["a.txt"]);
    assert_eq!(undone.recreated, ["b.txt"]);
    assert_eq!(undone.removed, ["new.txt"]);
    assert_eq!(repo.read("a.txt"), "one\ntwo\n");
    assert_eq!(repo.read("b.txt"), "bee\n");
    assert!(!repo.0.join("new.txt").exists());
    assert_eq!(repo.read("debug.log"), "noise\n", "ignored files are left alone");

    // One level only, and the conversation knows.
    assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
    assert!(r.engine.last_changes().is_none() && r.engine.last_diff(None).is_none());
    let note = r.engine.sessions.catch_up(Claude, "", 10_000).expect("even the model that ran the turn is told");
    assert!(note.contains("the user undid the changes of the previous turn"), "{note}");
}

#[tokio::test]
async fn undo_refuses_after_a_later_edit_and_keeps_the_summary_for_a_retry() {
    let repo = Repo::new("later", true);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, EDITS);
    r.turn("edit things", want(TaskType::Feature)).await;
    std::fs::write(repo.0.join("a.txt"), "one\ntwo\nthree\nmine\n").unwrap();
    assert_eq!(r.engine.undo_last(), Err(UndoError::ChangedSince(vec!["a.txt".into()])));
    assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\nmine\n");
    assert!(!repo.0.join("b.txt").exists(), "nothing was undone");
    assert_eq!(r.engine.sessions.turns(), 1, "a refused undo is not recorded");
    assert!(r.engine.last_changes().is_some());
}

#[tokio::test]
async fn a_turn_that_changes_nothing_has_no_summary_and_leaves_nothing_to_undo() {
    let repo = Repo::new("nothing", true);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, EDITS);
    r.turn("edit things", want(TaskType::Feature)).await;
    assert!(r.engine.last_changes().is_some());
    r.script(Claude, OK);
    let report = r.turn("just explain", want(TaskType::Feature)).await;
    assert!(report.changes.is_none());
    assert!(r.engine.last_changes().is_none(), "undo refers to the previous turn, which changed nothing");
    assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
}

#[tokio::test]
async fn a_turn_that_never_started_a_model_keeps_the_earlier_undo() {
    let repo = Repo::new("noroute", true);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, EDITS);
    r.turn("edit things", want(TaskType::Feature)).await;
    r.engine.runtime.cool_down(Claude, None, NOW);
    let report = r.turn("more", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute);
    assert!(r.engine.last_changes().is_some());
}

#[tokio::test]
async fn outside_a_git_repository_the_turn_works_and_just_has_no_changes() {
    let repo = Repo::new("nogit", false);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, EDITS);
    let report = r.turn("edit things", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.text, "edited");
    assert!(report.changes.is_none() && r.engine.last_changes().is_none());
    assert_eq!(r.engine.last_diff(None), None);
    assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
    assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
}

#[tokio::test]
async fn change_tracking_can_be_switched_off() {
    let repo = Repo::new("off", true);
    let mut r = tracked(&repo, &[Claude]);
    r.engine = r.engine.with_track_changes(false);
    r.script(Claude, EDITS);
    assert!(r.turn("edit things", want(TaskType::Feature)).await.changes.is_none());
}

#[tokio::test]
async fn a_cancelled_turn_still_reports_the_files_it_got_to_and_they_can_be_undone() {
    let repo = Repo::new("cancel-edits", true);
    let mut r = tracked(&repo, &[Claude]);
    r.script(Claude, "echo partial > half.txt; echo 'TOOL:Write half.txt'; sleep 30");
    let cancel = Cancel::new();
    r.ui.cancel_on_tool = Some(cancel.clone());
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    let report = r.engine.run_judged_cancellable("start it", judged, &mut r.ui, &cancel).await;
    assert_eq!(report.status, TurnStatus::Cancelled);
    let files = report.changes.expect("a half-done turn is the one you most want to undo").files;
    assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["half.txt"]);
    r.engine.undo_last().unwrap();
    assert!(!repo.0.join("half.txt").exists());
}
