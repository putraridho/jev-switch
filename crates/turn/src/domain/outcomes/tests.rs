use super::*;
use jev_shared::domain::acceptance::Rejection;
use jev_shared::domain::audit::Verdict;

const M: &str = "claude · claude-sonnet-5-5";

fn q(task: &str, acceptance: Acceptance, tokens: u64) -> Quality {
    Quality { task: task.into(), acceptance, spent_tokens: tokens, ..Quality::default() }
}

fn rejected() -> Acceptance {
    Acceptance::Rejected(Rejection::VerificationFailed)
}

#[test]
fn counts_by_model_and_task_and_never_calls_an_unverified_turn_accepted() {
    let mut o = Outcomes::default();
    o.add_turn("/p", M, true, &q("feature", Acceptance::Accepted, 1000));
    o.add_turn("/p", M, true, &q("feature", Acceptance::Unverified, 500));
    o.add_turn("/p", M, true, &q("docs", Acceptance::Unverified, 500));
    o.add_turn("/p", M, true, &q("feature", rejected(), 3000));
    let rows: Vec<_> = o.iter().map(|(m, t, tally)| (m.to_string(), t.to_string(), tally.accepted, tally.rejected, tally.unverified)).collect();
    assert_eq!(rows, [(M.to_string(), "docs".into(), 0, 0, 1), (M.to_string(), "feature".into(), 1, 1, 1)]);
    let t = o.iter().find(|r| r.1 == "feature").unwrap().2;
    assert_eq!(t.verified(), 2);
    assert_eq!(t.tokens_per_accepted(), Some(4000), "the failed turn's spend counts against the accepted one");
    assert!(!t.enough());
}

#[test]
fn a_turn_without_an_answer_is_never_counted_as_accepted() {
    let mut o = Outcomes::default();
    o.add_turn("/p", M, false, &q("feature", Acceptance::Accepted, 10));
    assert_eq!(o.by_model()[M].accepted, 0);
    o.add_turn("/p", "", true, &q("feature", Acceptance::Accepted, 10));
    assert_eq!(o.by_model().len(), 1);
}

#[test]
fn rework_after_a_revert_takes_the_earlier_acceptance_back_in_the_same_project_only() {
    let mut o = Outcomes::default();
    o.add_turn("/p", M, true, &q("feature", Acceptance::Accepted, 100));
    o.add_turn("/other", M, true, &q("feature", Acceptance::Accepted, 100));
    let redo = Quality { rework: Rework::AfterRevert, ..q("feature", Acceptance::Accepted, 100) };
    o.add_turn("/p", M, true, &redo);
    let t = &o.by_model()[M];
    assert_eq!((t.accepted, t.rejected, t.rework), (2, 1, 1));
}

#[test]
fn routing_gets_records_only_for_known_models_and_tasks() {
    let mut o = Outcomes::default();
    for _ in 0..6 {
        o.add_turn("/p", M, true, &q("feature", Acceptance::Accepted, 100));
    }
    o.add_turn("/p", "claude · x", true, &q("not-a-task", Acceptance::Accepted, 1));
    o.add_turn("/p", "garbled", true, &q("feature", Acceptance::Accepted, 1));
    let r = o.records();
    let spec = jev_catalog::domain::model_table::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap();
    let rec = r.get(spec, TaskType::Feature).unwrap();
    assert_eq!((rec.verified, rec.accepted, rec.tokens_per_accepted), (6, 6, Some(100)));
    assert!(rec.enough());
}

#[test]
fn it_reads_the_audit_entries() {
    let turn = |outcome, quality| Entry {
        ts_ms: 0,
        project: "/p".into(),
        event: Event::Turn {
            model: M.into(),
            outcome,
            measurement: jev_shared::Measurement::Reported,
            elapsed_ms: 1,
            in_tokens: 1,
            out_tokens: 1,
            prompt: String::new(),
            cached_tokens: 0,
            context_tokens: 0,
            tool_calls: 0,
            verdict: None,
            quality,
        },
    };
    let entries = [turn(Outcome::Answered, q("feature", Acceptance::Accepted, 5)), turn(Outcome::Failed, q("feature", Acceptance::Unverified, 5)), Entry { ts_ms: 0, project: "/p".into(), event: Event::Profile { name: "x".into() } }];
    let t = Outcomes::from_entries(&entries).by_model()[M].clone();
    assert_eq!((t.turns, t.accepted, t.unverified), (2, 1, 1));
}

#[test]
fn an_old_line_with_only_a_verdict_still_counts() {
    let line = |verdict| Entry {
        ts_ms: 0,
        project: "/p".into(),
        event: Event::Turn { model: M.into(), outcome: Outcome::Answered, measurement: jev_shared::Measurement::Reported, elapsed_ms: 1, in_tokens: 1, out_tokens: 1, prompt: String::new(), cached_tokens: 0, context_tokens: 0, tool_calls: 0, verdict, quality: Quality::default() },
    };
    let t = Outcomes::from_entries(&[line(Some(Verdict::Passed)), line(Some(Verdict::Failed)), line(None)]).by_model()[M].clone();
    assert_eq!((t.accepted, t.rejected, t.unverified), (1, 1, 1));
}
