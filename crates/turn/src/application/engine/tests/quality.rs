//! Checking a turn, judging whether its work was accepted, and what follows from that.

use super::*;
use jev_orchestration::application::ports::{CheckRun, Checks, QuickChecks};
use jev_shared::domain::acceptance::{Acceptance, Rejection, Rework};
use crate::domain::outcomes::Outcomes;
use crate::domain::verify_mode::VerifyMode;

/// A fake project check: answers from a queue (the last answer repeats) and records what it ran.
struct FakeChecks {
    runs: Mutex<Vec<String>>,
    answers: Mutex<Vec<CheckRun>>,
    quick: Option<&'static str>,
}

impl FakeChecks {
    fn new(answers: Vec<CheckRun>, quick: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self { runs: Mutex::new(Vec::new()), answers: Mutex::new(answers), quick })
    }
    fn ran(&self) -> Vec<String> {
        self.runs.lock().unwrap().clone()
    }
}

impl Checks for FakeChecks {
    fn test_command(&self, _dir: &Path) -> Option<String> {
        None
    }
    fn run(&self, _dir: &Path, command: &str, _stop: &dyn Fn() -> bool) -> CheckRun {
        self.runs.lock().unwrap().push(command.to_string());
        let mut answers = self.answers.lock().unwrap();
        if answers.len() > 1 { answers.remove(0) } else { answers[0].clone() }
    }
}

impl QuickChecks for FakeChecks {
    fn quick_command(&self, _dir: &Path) -> Option<String> {
        self.quick.map(str::to_string)
    }
}

const EDIT_A: &str = "echo more >> a.txt; printf 'FIN:edited\\n'";

fn rig_with_checks(repo: &Repo, mode: VerifyMode, checks: &Arc<FakeChecks>) -> Rig {
    let mut r = rig_in(repo.0.clone(), &states_for(&[Claude]), EDIT_A);
    r.engine = r.engine.with_track_changes(true).with_verify(mode, Some(checks.clone()), Some(checks.clone()));
    r
}

fn failing() -> CheckRun {
    CheckRun::Failed { output: "boom: a.txt does not compile".into() }
}

#[tokio::test]
async fn only_a_passing_check_after_changes_accepts_a_turn() {
    let repo = Repo::new("accept", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("make check".into()), &checks);
    let report = r.turn("edit a", want(TaskType::Feature)).await;
    assert_eq!(report.quality.acceptance, Acceptance::Accepted);
    assert_eq!((report.quality.task.as_str(), report.quality.retries, report.quality.spent_tokens), ("feature", 0, 7));
    assert_eq!(checks.ran(), ["make check"]);
    let t = &r.engine.outcomes().by_model()["claude · claude-sonnet-5-5"];
    assert_eq!((t.accepted, t.rejected), (1, 0));
}

#[tokio::test]
async fn an_answer_that_changed_files_but_was_not_checked_is_unverified() {
    let repo = Repo::new("unchecked", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Off, &checks);
    let report = r.turn("edit a", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.quality.acceptance, Acceptance::Unverified);
    assert!(checks.ran().is_empty());
}

#[tokio::test]
async fn a_failed_check_rejects_the_turn_and_is_told_to_the_next_turn_once() {
    let repo = Repo::new("failed", true);
    let checks = FakeChecks::new(vec![failing(), CheckRun::Passed], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("make check".into()), &checks);
    let first = r.turn("edit a", want(TaskType::Feature)).await;
    assert_eq!(first.quality.acceptance, Acceptance::Rejected(Rejection::VerificationFailed));
    assert!(r.ui.notes.iter().any(|n| n.contains("failed")), "{:?}", r.ui.notes);

    let second = r.turn("fix it", want(TaskType::Feature)).await;
    let prompts: Vec<String> = r.started().into_iter().map(|s| s.prompt).collect();
    assert!(prompts[1].contains("[verification failed]") && prompts[1].contains("boom: a.txt"), "{}", prompts[1]);
    assert!(prompts[1].contains("fix it"));
    assert!(!prompts[0].contains("verification failed"));
    // The same files again after a failed check is rework, and passing now accepts it.
    assert_eq!((second.quality.rework, second.quality.acceptance), (Rework::AfterFailedCheck, Acceptance::Accepted));

    r.turn("one more", want(TaskType::Feature)).await;
    assert!(!r.started()[2].prompt.contains("verification failed"), "a passed check clears the warning");
}

#[tokio::test]
async fn a_failure_waits_for_a_turn_that_actually_starts_a_model() {
    let repo = Repo::new("kept", true);
    let checks = FakeChecks::new(vec![failing()], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("c".into()), &checks);
    r.turn("edit a", want(TaskType::Feature)).await;
    r.engine.runtime.cool_down(Claude, None, NOW);
    assert_eq!(r.turn("again", want(TaskType::Feature)).await.status, TurnStatus::NoRoute);
    r.clock.fetch_add(48 * 3_600_000, Ordering::SeqCst);
    r.turn("again", want(TaskType::Feature)).await;
    assert!(r.started().last().unwrap().prompt.contains("[verification failed]"));
}

#[tokio::test]
async fn nothing_is_run_in_plan_mode_or_when_nothing_changed() {
    let repo = Repo::new("never", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("c".into()), &checks);
    r.engine.set_mode(Mode::Plan);
    r.turn("edit a", want(TaskType::Feature)).await;
    assert!(checks.ran().is_empty(), "plan mode never runs a check");
    r.engine.set_mode(Mode::Edit);
    r.script(Claude, OK);
    let unchanged = r.turn("explain", want(TaskType::Explain)).await;
    assert!(checks.ran().is_empty());
    assert_eq!(unchanged.quality.acceptance, Acceptance::Unverified);
}

#[tokio::test]
async fn auto_runs_the_cheap_check_the_project_has_and_says_when_it_has_none() {
    let repo = Repo::new("auto", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], Some("cargo check"));
    let mut r = rig_with_checks(&repo, VerifyMode::Auto, &checks);
    assert_eq!(r.turn("edit a", want(TaskType::Feature)).await.quality.acceptance, Acceptance::Accepted);
    assert_eq!(checks.ran(), ["cargo check"]);

    let none = FakeChecks::new(vec![CheckRun::Passed], None);
    let repo2 = Repo::new("auto-none", true);
    let mut r = rig_with_checks(&repo2, VerifyMode::Auto, &none);
    let report = r.turn("edit a", want(TaskType::Feature)).await;
    assert!(none.ran().is_empty());
    assert_eq!(report.quality.acceptance, Acceptance::Unverified);
    assert!(r.ui.notes.iter().any(|n| n.contains("no cheap check")), "{:?}", r.ui.notes);
}

#[tokio::test]
async fn a_check_that_did_not_decide_accepts_nothing() {
    let repo = Repo::new("timeout", true);
    let checks = FakeChecks::new(vec![CheckRun::TimedOut { after: Duration::from_secs(300) }], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("c".into()), &checks);
    let report = r.turn("edit a", want(TaskType::Feature)).await;
    assert_eq!(report.quality.acceptance, Acceptance::Unverified);
    r.turn("again", want(TaskType::Feature)).await;
    assert!(!r.started()[1].prompt.contains("verification failed"));
}

#[tokio::test]
async fn redoing_a_turn_the_person_undid_is_rework_and_takes_the_acceptance_back() {
    let repo = Repo::new("revert", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], None);
    let mut r = rig_with_checks(&repo, VerifyMode::Command("c".into()), &checks);
    r.turn("edit a", want(TaskType::Feature)).await;
    r.engine.undo_last().unwrap();
    let redo = r.turn("edit a again", want(TaskType::Feature)).await;
    assert_eq!(redo.quality.rework, Rework::AfterRevert);
    let t = &r.engine.outcomes().by_model()["claude · claude-sonnet-5-5"];
    assert_eq!((t.accepted, t.rejected, t.rework), (1, 1, 1));
}

#[tokio::test]
async fn failovers_are_counted_as_retries_and_their_spend_is_kept() {
    let repo = Repo::new("retry", true);
    let checks = FakeChecks::new(vec![CheckRun::Passed], None);
    let mut r = rig_in(repo.0.clone(), &states_for(&[Claude, Codex]), EDIT_A);
    r.engine = r.engine.with_track_changes(true).with_verify(VerifyMode::Command("c".into()), Some(checks.clone()), None);
    let first = first_choice(TaskType::Feature, &[Claude, Codex]);
    r.script(first, QUOTA);
    let report = r.turn("edit a", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.quality.retries, 1);
    assert_eq!(report.quality.acceptance, Acceptance::Accepted);
}

fn records_favouring(model: &str, against: &str) -> Outcomes {
    let mut o = Outcomes::default();
    for _ in 0..10 {
        let good = jev_shared::domain::acceptance::Quality { task: "feature".into(), acceptance: Acceptance::Accepted, spent_tokens: 100, ..Default::default() };
        let bad = jev_shared::domain::acceptance::Quality { acceptance: Acceptance::Rejected(Rejection::VerificationFailed), ..good.clone() };
        o.add_turn("/p", model, true, &good);
        o.add_turn("/p", against, true, &bad);
    }
    o
}

#[tokio::test]
async fn accepted_outcomes_reorder_equal_tier_models_only_when_learning_is_on() {
    let ready = [Claude, Codex];
    let static_first = first_choice(TaskType::Feature, &ready);
    let (better, worse) = if static_first == Claude { ("codex · gpt-6.1-terra", "claude · claude-sonnet-5-5") } else { ("claude · claude-sonnet-5-5", "codex · gpt-6.1-terra") };
    let outcomes = records_favouring(better, worse);

    let mut off = rig(&ready);
    off.engine = off.engine.with_outcomes(outcomes.clone(), false);
    assert_eq!(off.turn(LONG, new_task(TaskType::Feature)).await.provider, Some(static_first));

    let mut on = rig(&ready);
    on.engine = on.engine.with_outcomes(outcomes, true);
    let report = on.turn(LONG, new_task(TaskType::Feature)).await;
    assert_eq!(report.provider.map(|p| p.to_string()), Some(better.split(' ').next().unwrap().to_string()));
    assert!(on.ui.routes.iter().any(|l| l.contains("accepted work")), "{:?}", on.ui.routes);
}

#[tokio::test]
async fn a_pin_still_wins_over_learned_outcomes() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    r.engine = r.engine.with_outcomes(records_favouring("codex · gpt-6.1-terra", "claude · claude-sonnet-5-5"), true);
    r.engine.set_pin(Pin::Model(Claude, "claude-sonnet-5-5".into()));
    assert_eq!(r.turn(LONG, new_task(TaskType::Feature)).await.provider, Some(Claude));
}
