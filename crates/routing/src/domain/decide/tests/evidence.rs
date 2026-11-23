//! Observed accepted work reorders equal-tier neighbours and nothing else.

use super::*;
use crate::domain::track_record::{TrackRecord, TrackRecords};

fn good(accepted: u32) -> TrackRecord {
    TrackRecord { verified: 10, accepted, tokens_per_accepted: Some(1_000) }
}

/// Feature chain, everything usable: static first choice is claude-sonnet-5-5 and the next
/// models are of the same tier (see the catalog chains).
fn records_for(first_wins: bool) -> TrackRecords {
    let chain = default_chain(TaskType::Feature, false);
    let mut r = TrackRecords::default();
    for (i, s) in chain.iter().enumerate().filter(|(_, s)| s.tier == chain[0].tier) {
        let accepted = if (i == 0) == first_wins { 10 } else { 3 };
        r.insert(s.provider, s.id, TaskType::Feature, good(accepted));
    }
    r
}

fn decide_with(f: &mut Fixture, records: Option<TrackRecords>) -> Result<RouteDecision, NoRoute> {
    f.records = records;
    f.run(&mut RoutingState::default(), Some(&sig(TaskType::Feature)), NEW_TASK_WORDS)
}

#[test]
fn without_records_the_static_first_choice_stands() {
    let mut f = Fixture::new(&PROVIDERS);
    let base = decide_with(&mut f, None).unwrap();
    assert_eq!(base.spec.id, "claude-sonnet-5-5");
    assert_eq!(decide_with(&mut f, Some(TrackRecords::default())).unwrap().spec.id, base.spec.id);
}

#[test]
fn a_better_record_among_equal_tier_models_wins_and_says_so() {
    let mut f = Fixture::new(&PROVIDERS);
    let chain = default_chain(TaskType::Feature, false);
    let tier = chain[0].tier;
    assert!(chain.iter().filter(|s| s.tier == tier).count() >= 2, "the Feature chain needs two models of its first tier for this test");
    let got = decide_with(&mut f, Some(records_for(false))).unwrap();
    assert_ne!(got.spec.id, chain[0].id, "{}", got.reason);
    assert_eq!(got.spec.tier, tier);
    assert!(got.reason.contains("accepted work"), "{}", got.reason);
    // The static winner with the better record keeps its place.
    assert_eq!(decide_with(&mut f, Some(records_for(true))).unwrap().spec.id, chain[0].id);
}

#[test]
fn a_pin_a_user_chain_and_quota_steering_outrank_the_records() {
    let chain = default_chain(TaskType::Feature, false);
    let records = records_for(false);
    // A pin is exact.
    let mut f = Fixture::new(&PROVIDERS);
    f.pin = Pin::Model(Claude, "claude-sonnet-5-5".into());
    assert_eq!(decide_with(&mut f, Some(records.clone())).unwrap().spec.id, "claude-sonnet-5-5");
    // The user's own chain is not reordered.
    let mut f = Fixture::new(&PROVIDERS);
    f.overrides = Overrides::from_raw(vec![("feature".into(), catalog::RawChain::of(&["claude:claude-sonnet-5-5", "codex:gpt-6.1-terra"]))]);
    assert_eq!(decide_with(&mut f, Some(records.clone())).unwrap().spec.id, "claude-sonnet-5-5");
    // A nearly spent Claude plan steers off Claude even if Claude has the best record.
    let mut f = Fixture::new(&PROVIDERS);
    f.plan = Some(0.95);
    let got = decide_with(&mut f, Some(records_for(true))).unwrap();
    assert_ne!(got.spec.provider, Claude, "{}", got.reason);
    let _ = chain;
}

#[test]
fn an_unusable_better_model_is_never_picked() {
    // Only Claude can run: whatever the records prefer, an unavailable model is not chosen.
    let mut f = Fixture::new(&[Claude]);
    let got = decide_with(&mut f, Some(records_for(false))).unwrap();
    assert_eq!(got.spec.provider, Claude);
}

#[test]
fn security_work_never_follows_the_records() {
    let chain = default_chain(TaskType::Security, true);
    let mut r = TrackRecords::default();
    for (i, s) in chain.iter().enumerate() {
        r.insert(s.provider, s.id, TaskType::Security, good(if i == 0 { 0 } else { 10 }));
    }
    let mut f = Fixture::new(&PROVIDERS);
    f.records = Some(r);
    let with = f.run(&mut RoutingState::default(), Some(&sig(TaskType::Security)), NEW_TASK_WORDS).unwrap();
    f.records = None;
    let without = f.run(&mut RoutingState::default(), Some(&sig(TaskType::Security)), NEW_TASK_WORDS).unwrap();
    assert_eq!(with.spec.id, without.spec.id);
}
