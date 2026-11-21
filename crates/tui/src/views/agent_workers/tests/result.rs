use super::*;
use crate::views::agent_workers::result::*;

#[test]
fn result_card_for_a_finished_worker_offers_review_accept_discard() {
    let mut x = w("w1", WorkerState::Done);
    x.files = vec!["a.rs".into(), "b.rs".into()];
    let c = result_card(&ResultFacts { worker: x, summary: "Added tests".into(), tests: Some("12 passed".into()), error: None }, 80, OFF);
    for want in ["done", "Added tests", "files (2): a.rs, b.rs", "tests: 12 passed", "$0.04", "[r] review", "[a] accept", "[d] discard"] {
        assert!(c.contains(want), "missing {want:?} in {c}");
    }
}

#[test]
fn result_card_for_a_failure_gives_the_reason_and_next_step() {
    let c = result_card(&ResultFacts { worker: w("w1", WorkerState::Failed), summary: String::new(), tests: None, error: Some("limit hit".into()) }, 80, OFF);
    assert!(c.contains("reason: limit hit") && c.contains("[t] retry") && !c.contains("accept"), "{c}");
}

#[test]
fn result_card_cleans_hostile_worker_text() {
    let c = result_card(&ResultFacts { worker: w("w1", WorkerState::Done), summary: "ok\x1b[2J\x1b]0;x\x07 done".into(), tests: None, error: None }, 80, OFF);
    assert!(!c.contains('\x1b') && !c.contains('\x07'));
}
