//! Holding a downgrade for the cache and recording the turn that ran.

use super::*;

#[test]
fn hold_steps_down_after_max_hold_and_upgrade_is_immediate() {
    let f = Fixture::new(&[Claude]);
    let easy = sig(TaskType::Feature); // chain wants Sonnet
    let mut st = on("claude-opus-5-5");
    for turn in 1..MAX_HOLD {
        let d = f.run(&mut st, Some(&easy), 3).unwrap();
        assert_eq!(d.spec.id, "claude-opus-5-5", "turn {turn} should hold");
        assert!(d.reason.contains("holding"), "{}", d.reason);
        record_turn(&mut st, d.spec);
    }
    let d = f.run(&mut st, Some(&easy), 3).unwrap();
    assert_eq!(d.spec.id, "claude-sonnet-5-5");
    assert!(d.reason.contains("stepped down"), "{}", d.reason);
    record_turn(&mut st, d.spec);
    assert_eq!(st.held, 0);
    // Upgrade is immediate and resets the counter.
    let mut st = on("claude-sonnet-5-5");
    let d = f.run(&mut st, Some(&sig(TaskType::HardDebug)), 3).unwrap();
    assert_eq!(d.spec.id, "claude-opus-5-5");
    assert!(d.reason.contains("upgrade"), "{}", d.reason);
    // A hard turn in the middle of a hold restarts it.
    let mut st = on("claude-opus-5-5");
    f.run(&mut st, Some(&easy), 3).unwrap();
    f.run(&mut st, Some(&sig(TaskType::HardDebug)), 3).unwrap();
    assert_eq!(st.held, 0);
}

#[test]
fn step_down_is_one_tier_and_stays_on_the_chain_and_usable() {
    let o = catalog::Overrides::from_raw(vec![(
        "simple_edit".to_string(),
        catalog::RawChain::of(&["claude:claude-haiku-5-5", "claude:claude-sonnet-5-5", "claude:claude-opus-5-5"]),
    )]);
    // Chain order puts Haiku first, but from Opus the step is to Sonnet, not Haiku.
    let mut f = Fixture::new(&[Claude]);
    f.overrides = o;
    let mut st = on("claude-opus-5-5");
    st.held = MAX_HOLD - 1;
    let d = f.run(&mut st, Some(&sig(TaskType::SimpleEdit)), 3).unwrap();
    assert_eq!(d.spec.id, "claude-sonnet-5-5");
    // Sonnet unusable: the next best servable entry is Haiku.
    f.usable = Usable::new([(Claude, "claude-haiku-5-5"), (Claude, "claude-opus-5-5")]);
    let mut st = on("claude-opus-5-5");
    st.held = MAX_HOLD - 1;
    assert_eq!(f.run(&mut st, Some(&sig(TaskType::SimpleEdit)), 3).unwrap().spec.id, "claude-haiku-5-5");
}

#[test]
fn record_turn_updates_current_and_resets_hold_on_a_model_change() {
    let mut st = RoutingState::default();
    let sonnet = catalog::find(Claude, "claude-sonnet-5-5").unwrap();
    record_turn(&mut st, sonnet);
    assert_eq!(st.current, Some((Claude, "claude-sonnet-5-5".to_string())));
    st.held = 2;
    record_turn(&mut st, sonnet);
    assert_eq!(st.held, 2);
    record_turn(&mut st, catalog::find(Codex, "gpt-6.1-sol").unwrap());
    assert_eq!((st.current.clone(), st.held), (Some((Codex, "gpt-6.1-sol".to_string())), 0));
}

#[test]
fn staying_warm_reports_a_drop_only_below_what_the_task_needs() {
    // Explain, Claude warm on Sonnet, Grok 4.7 (Top) also ready: a short follow-up
    // stays on Sonnet, which is all explaining needs, so it is not "lower tier".
    let f = Fixture::new(&[Claude, Grok]);
    let mut st = RoutingState { current: Some((Claude, "claude-sonnet-5-5".into())), ..Default::default() };
    let d = f.run(&mut st, Some(&sig(TaskType::Explain)), 3).unwrap();
    assert_eq!(d.spec.id, "claude-sonnet-5-5", "{}", d.reason);
    assert!(!d.dropped_tier, "{}", d.reason);
}
