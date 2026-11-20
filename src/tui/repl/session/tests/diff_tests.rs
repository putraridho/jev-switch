//! `/diff`, `/undo`, `/rewind` and the hunk review.

use super::*;
use crate::tui::command::Command;

#[tokio::test]
async fn diff_shows_the_turn_and_undo_asks_first_and_refuses_without_a_terminal() {
    let mut r = rig("undo", ANSWER);
    let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
    r.repl.finish(&report, elapsed);
    r.out.clear();

    run(&mut r, "/diff").await;
    let d = r.out.text();
    assert!(d.contains("+ two") && d.contains("a.txt"), "{d}");
    r.out.clear();
    run(&mut r, "/diff nothing.txt").await;
    assert!(r.out.text().contains("No changes to show for nothing.txt") || r.out.text().contains("No differences"), "{}", r.out.text());

    // No terminal: it lists what it would do and refuses.
    r.out.clear();
    run(&mut r, "/undo").await;
    let u = r.out.text();
    assert!(u.contains("Would revert 1 file") && u.contains("a.txt") && u.contains("Not undone: asking needs a terminal"), "{u}");
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n", "nothing was touched");

    // A "no" leaves it, a "yes" puts the file back.
    let asked = Arc::new(Mutex::new(Vec::new()));
    for (answer, expect_reverted) in [(false, false), (true, true)] {
        r.out.clear();
        let mut int = Presses { pending: vec![] };
        let mut yes = Answer(answer, asked.clone());
        let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
        r.repl.handle(Command::Undo, &mut io).await;
        let content = std::fs::read_to_string(r.dir.join("a.txt")).unwrap();
        assert_eq!(content == "one\n", expect_reverted, "{content:?} / {}", r.out.text());
    }
    assert_eq!(asked.lock().unwrap().as_slice(), ["Revert 1 file? [y/N] ", "Revert 1 file? [y/N] "]);
    assert!(r.out.text().contains("✓ reverted: restored a.txt"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/undo").await;
    assert!(r.out.text().contains("Nothing to undo"), "{}", r.out.text());
}

async fn two_edit_turns(name: &str) -> Rig {
    let mut r = rig(name, ANSWER);
    for prompt in ["change a", "change a again"] {
        let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, prompt).await else { panic!() };
        r.repl.finish(&report, elapsed);
    }
    r.out.clear();
    r
}

async fn run_with(r: &mut Rig, line: &str, answer: bool) -> Vec<String> {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut int = Presses { pending: vec![] };
    let mut yes = Answer(answer, asked.clone());
    let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
    r.repl.handle(crate::tui::command::parse(line), &mut io).await;
    let questions = asked.lock().unwrap().clone();
    questions
}

#[tokio::test]
async fn diff_looks_at_one_turn_or_all_of_them_with_line_numbers() {
    let mut r = two_edit_turns("diff-scopes").await;
    run(&mut r, "/diff --list").await;
    let l = r.out.text();
    assert!(l.contains(" 1 a.txt") && l.contains(" 2 a.txt"), "{l}");

    r.out.clear();
    run(&mut r, "/diff --all").await;
    let all = r.out.text();
    assert!(all.contains("▸ a.txt  +2 −0") && all.contains("+ two"), "{all}");

    r.out.clear();
    run(&mut r, "/diff --turn 1").await;
    let one = r.out.text();
    assert!(one.contains("▸ a.txt  +1 −0") && one.contains("  1 1   one") && one.contains("2 + two"), "{one}");

    r.out.clear();
    run(&mut r, "/diff --turn 9").await;
    assert!(r.out.text().contains("there is no turn 9"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/diff --bogus").await;
    assert!(r.out.text().contains("unknown option --bogus"), "{}", r.out.text());
}

#[tokio::test]
async fn diff_exports_a_patch_that_git_applies() {
    let mut r = two_edit_turns("diff-export").await;
    run(&mut r, "/diff --all --export all.patch").await;
    assert!(r.out.text().contains("patch written to"), "{}", r.out.text());
    let ok = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&r.dir).args(args).status().unwrap().success();
    assert!(ok(&["checkout", "--", "a.txt"]));
    assert!(ok(&["apply", "all.patch"]));
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");
    r.out.clear();
    run(&mut r, "/diff --all --export all.patch").await;
    assert!(r.out.text().contains("already exists"), "{}", r.out.text());
}

#[tokio::test]
async fn review_reverts_only_the_hunks_the_user_rejects() {
    let mut r = two_edit_turns("diff-review").await;
    run(&mut r, "/diff --review").await;
    assert!(r.out.text().contains("needs a terminal"), "{}", r.out.text());

    r.out.clear();
    let asked = run_with(&mut r, "/diff --review", false).await;
    assert_eq!(asked, ["Revert this hunk? [y/N] "]);
    assert!(r.out.text().contains("hunk 1 of 1") && r.out.text().contains("1 kept, 0 reverted"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

    r.out.clear();
    run_with(&mut r, "/diff --review", true).await;
    assert!(r.out.text().contains("0 kept, 1 reverted") && !r.out.text().contains("/undo will refuse"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n");
}

#[tokio::test]
async fn undo_and_rewind_still_work_after_a_hunk_was_reverted_in_review() {
    let mut r = two_edit_turns("review-then-undo").await;
    run_with(&mut r, "/diff --review", true).await;
    r.out.clear();
    run_with(&mut r, "/undo", true).await;
    assert!(r.out.text().contains("reverted:") && !r.out.text().contains("Not undone"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n");

    let mut r = two_edit_turns("review-then-rewind").await;
    run_with(&mut r, "/diff --review", true).await;
    r.out.clear();
    run_with(&mut r, "/rewind 1", true).await;
    assert!(r.out.text().contains("rewound: 2 turn(s) undone"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\n");
}

#[tokio::test]
async fn rewind_lists_turns_and_undoes_from_the_chosen_one() {
    let mut r = two_edit_turns("rewind").await;
    run(&mut r, "/rewind").await;
    assert!(r.out.text().contains(" 1 a.txt") && r.out.text().contains("/rewind <number>"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/rewind 5").await;
    assert!(r.out.text().contains("There is no turn 5"), "{}", r.out.text());

    r.out.clear();
    run(&mut r, "/rewind 1").await;
    assert!(r.out.text().contains("Would undo 2 turns") && r.out.text().contains("needs a terminal"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

    assert_eq!(run_with(&mut r, "/rewind 1", false).await, ["Rewind to before turn 1? [y/N] "]);
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

    r.out.clear();
    run_with(&mut r, "/rewind 2", true).await;
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n", "only the second turn is undone");
    r.out.clear();
    run_with(&mut r, "/rewind 1", true).await;
    assert!(r.out.text().contains("rewound: 1 turn(s) undone"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\n");
    r.out.clear();
    run(&mut r, "/diff --all").await;
    assert!(r.out.text().contains("not changed any files"), "{}", r.out.text());
}

#[tokio::test]
async fn undo_that_would_lose_later_edits_explains_instead_of_failing() {
    let mut r = rig("undo-late", ANSWER);
    let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
    r.repl.finish(&report, elapsed);
    std::fs::write(r.dir.join("a.txt"), "my own edit\n").unwrap();
    r.out.clear();
    let mut int = Presses { pending: vec![] };
    let mut yes = Answer(true, Arc::new(Mutex::new(Vec::new())));
    let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
    r.repl.handle(Command::Undo, &mut io).await;
    let o = r.out.text();
    assert!(o.contains("✗ Not undone: those files changed since the turn") && o.contains("a.txt") && o.contains("`/diff`"), "{o}");
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "my own edit\n");
}

