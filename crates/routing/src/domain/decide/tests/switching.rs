//! When a turn may change provider: the gate, the warm cache and plan steering.

use super::*;

#[test]
fn hard_debug_continuation_upgrades_gemini_before_provider_stickiness() {
    let mut fixture = Fixture::new(&[Gemini, Codex]);
    let signals = Signals { task_confidence: 0.99, continues_p: 0.99, ..sig(TaskType::HardDebug) };
    fixture.now_ms = 60_000;
    for words in [3, NEW_TASK_WORDS] {
        let mut state = on("gemini-3.8-flash");
        state.held = MAX_HOLD - 1;
        state.warm = Some(WarmCache { provider: Gemini, tokens: 80_000, seen_ms: 0 });
        let decision = fixture.run(&mut state, Some(&signals), words).unwrap();
        assert_eq!(decision.spec.id, "gemini-4-neon", "{}", decision.reason);
        assert!(!decision.switched_provider);
        assert!(!decision.dropped_tier);
        assert_eq!(state.held, 0);
    }
    let boundary = Signals { continues_p: 0.0, task_confidence: 0.6, ..signals.clone() };
    assert_eq!(fixture.run(&mut on("gemini-3.8-flash"), Some(&boundary), NEW_TASK_WORDS).unwrap().spec.id, "gemini-4-neon");
    let confident = Signals { continues_p: 0.0, ..signals.clone() };
    fixture.now_ms = 400_000;
    assert_eq!(fixture.run(&mut on("gemini-3.8-flash"), Some(&confident), NEW_TASK_WORDS).unwrap().spec.id, "gpt-6.1-sol");
    let correction = Signals { correction_p: Some(0.99), ..signals };
    assert_eq!(fixture.run(&mut on("gemini-3.8-flash"), Some(&correction), 3).unwrap().spec.id, "gpt-6.1-sol");
    let security = Signals { security_keywords: Security::Strong, task_confidence: 0.4, continues_p: 0.99, ..sig(TaskType::Feature) };
    assert_eq!(fixture.run(&mut on("gemini-3.8-flash"), Some(&security), 3).unwrap().spec.id, "gpt-6.1-sol");
    fixture.pin = Pin::Model(Gemini, "gemini-3.8-flash".into());
    assert_eq!(fixture.run(&mut on("gemini-3.8-flash"), Some(&correction), 3).unwrap().spec.id, "gemini-3.8-flash");
}

#[test]
fn quality_floor_uses_an_adequate_same_provider_model_not_the_chain_head_tier() {
    let mut fixture = Fixture::new(&[Gemini, Codex]);
    fixture.overrides = Overrides::from_raw(vec![(
        "feature".into(),
        catalog::RawChain::of(&["codex:gpt-6.1-sol", "gemini:gemini-3.8-flash", "gemini:gemini-4-hydrogen"]),
    )]);
    for confidence in [0.4, 0.6, 0.99] {
        let signals = Signals { continues_p: 0.99, task_confidence: confidence, ..sig(TaskType::Feature) };
        let decision = fixture.run(&mut on("gemini-3.8-flash"), Some(&signals), 3).unwrap();
        assert_eq!(decision.spec.id, "gemini-4-hydrogen", "{}", decision.reason);
        assert!(!decision.switched_provider);
        assert!(!decision.dropped_tier);
    }
}

#[test]
fn quality_floor_switches_when_no_adequate_same_provider_model_can_serve() {
    let mut fixture = Fixture::new(&[Gemini, Codex]);
    let signals = Signals { task_confidence: 0.99, continues_p: 0.99, ..sig(TaskType::HardDebug) };
    fixture.usable = Usable::new([(Gemini, "gemini-3.8-flash"), (Codex, "gpt-6.1-sol")]);
    let mut state = on("gemini-3.8-flash");
    state.held = MAX_HOLD - 1;
    state.warm = Some(WarmCache { provider: Gemini, tokens: 80_000, seen_ms: 0 });
    let decision = fixture.run(&mut state, Some(&signals), 3).unwrap();
    assert_eq!(decision.spec.id, "gpt-6.1-sol", "{}", decision.reason);
    assert!(decision.switched_provider);
    assert!(!decision.dropped_tier);
    assert_eq!(state.held, 0);
}

#[test]
fn quality_floor_preserves_unavailable_tier_degradation() {
    let mut fixture = Fixture::new(&[Gemini]);
    fixture.overrides = Overrides::from_raw(vec![(
        "hard_debug".into(),
        catalog::RawChain::of(&["gemini:gemini-4-hydrogen", "gemini:gemini-4-neon"]),
    )]);
    fixture.usable = Usable::new([(Gemini, "gemini-3.8-flash"), (Gemini, "gemini-4-hydrogen")]);
    let signals = Signals { continues_p: 0.99, ..sig(TaskType::HardDebug) };
    let decision = fixture.run(&mut on("gemini-3.8-flash"), Some(&signals), 3).unwrap();
    assert_eq!(decision.spec.id, "gemini-4-hydrogen");
    assert!(decision.dropped_tier);
}

#[test]
fn plan_steering_overrides_continuation_and_cache_but_not_pins() {
    let mut fixture = Fixture::new(&[Claude, Codex]);
    let signals = Signals { continues_p: 0.99, ..sig(TaskType::Feature) };
    fixture.now_ms = 60_000;
    for utilization in [0.89, 0.90, 0.99] {
        fixture.plan = Some(utilization);
        for judgment in [Some(&signals), None] {
            for words in [3, NEW_TASK_WORDS] {
                let mut state = on("claude-sonnet-5-5");
                state.held = MAX_HOLD - 1;
                state.warm = Some(WarmCache { provider: Claude, tokens: 80_000, seen_ms: 0 });
                let decision = fixture.run(&mut state, judgment, words).unwrap();
                let steered = utilization >= 0.90;
                assert_eq!(decision.spec.provider, if steered { Codex } else { Claude }, "{}", decision.reason);
                assert_eq!(decision.switched_provider, steered);
                assert_eq!(decision.reason.contains("steered off claude"), steered);
                if steered {
                    assert_eq!(state.held, 0);
                }
            }
        }
    }
    for pin in [Pin::Provider(Claude), Pin::Model(Claude, "claude-sonnet-5-5".into())] {
        fixture.pin = pin;
        assert_eq!(fixture.run(&mut on("claude-sonnet-5-5"), Some(&signals), 3).unwrap().spec.provider, Claude);
    }
    fixture.pin = Pin::Auto;
    fixture.context_tokens = 600_000;
    assert_eq!(fixture.run(&mut on("claude-sonnet-5-5"), Some(&signals), 3).unwrap().spec.provider, Claude);
    fixture.context_tokens = 0;
    fixture.usable = Usable::all_of([Claude]);
    assert_eq!(fixture.run(&mut on("claude-sonnet-5-5"), Some(&signals), 3).unwrap().spec.provider, Claude);
}

#[test]
fn short_follow_up_and_continuation_stay_put() {
    let f = Fixture::new(&PROVIDERS);
    // SimpleEdit's best is DeepSeek, but the session is on Claude.
    let new_task = sig(TaskType::SimpleEdit);
    let d = f.run(&mut on("claude-sonnet-5-5"), Some(&new_task), 3).unwrap();
    assert_eq!(d.spec.id, "claude-sonnet-5-5");
    assert!(d.reason.contains("warm session"), "{}", d.reason);
    let continues = Signals { continues_p: 0.9, ..sig(TaskType::SimpleEdit) };
    assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&continues), NEW_TASK_WORDS).unwrap().spec.id, "claude-sonnet-5-5");
    let vague = Signals { task_confidence: 0.4, ..sig(TaskType::SimpleEdit) };
    assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&vague), NEW_TASK_WORDS).unwrap().spec.id, "claude-sonnet-5-5");
}

#[test]
fn correction_allows_a_switch() {
    let f = Fixture::new(&PROVIDERS);
    let corr = Signals { correction_p: Some(0.8), task_confidence: 0.55, ..sig(TaskType::SimpleEdit) };
    let d = f.run(&mut on("claude-sonnet-5-5"), Some(&corr), 3).unwrap();
    assert_eq!(d.spec.id, "deepseek-flash");
    assert!(d.switched_provider);
    let weak = Signals { correction_p: Some(0.3), ..corr };
    assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&weak), 3).unwrap().spec.id, "claude-sonnet-5-5");
}

#[test]
fn unconfident_boundary_keeps_the_provider_confident_switches() {
    let f = Fixture::new(&PROVIDERS);
    let unsure = Signals { task_confidence: 0.6, ..sig(TaskType::SimpleEdit) };
    let d = f.run(&mut on("claude-sonnet-5-5"), Some(&unsure), NEW_TASK_WORDS).unwrap();
    assert_eq!((d.spec.id, d.switched_provider), ("claude-sonnet-5-5", false));
    assert!(d.reason.contains("not confident enough"), "{}", d.reason);
    let sure = Signals { task_confidence: 0.8, ..sig(TaskType::SimpleEdit) };
    let d = f.run(&mut on("claude-sonnet-5-5"), Some(&sure), NEW_TASK_WORDS).unwrap();
    assert_eq!((d.spec.id, d.switched_provider), ("deepseek-flash", true));
    assert!(d.reason.contains("switched from claude"), "{}", d.reason);
    // Unconfident, but the current provider's best is weaker than the pick: switch anyway.
    let unsure_hard = Signals { task_confidence: 0.6, ..sig(TaskType::HardDebug) };
    let d = f.run(&mut on("claude-haiku-5-5"), Some(&unsure_hard), NEW_TASK_WORDS).unwrap();
    assert_eq!(d.spec.provider, Claude); // Opus is on the chain: provider kept, upgraded
    assert_eq!(d.spec.id, "claude-opus-5-5");
}

#[test]
fn a_big_warm_cache_keeps_the_provider_until_it_expires_or_a_rule_forces_the_switch() {
    let mut f = Fixture::new(&PROVIDERS);
    let sure = Signals { task_confidence: 0.8, ..sig(TaskType::SimpleEdit) };
    let warm = |tokens| {
        let mut s = on("claude-sonnet-5-5");
        s.warm = Some(WarmCache { provider: Claude, tokens, seen_ms: 0 });
        s
    };
    f.now_ms = 60_000;
    let d = f.run(&mut warm(80_000), Some(&sure), NEW_TASK_WORDS).unwrap();
    assert_eq!((d.spec.provider, d.switched_provider), (Claude, false));
    assert!(d.reason.contains("80K token cache is warm"), "{}", d.reason);
    // A small cache is cheap to rebuild.
    assert!(f.run(&mut warm(5_000), Some(&sure), NEW_TASK_WORDS).unwrap().switched_provider);
    // The window is over: nothing is left to lose.
    f.now_ms = 400_000;
    assert!(f.run(&mut warm(80_000), Some(&sure), NEW_TASK_WORDS).unwrap().switched_provider);
    // A likely correction switches whatever the cache holds.
    f.now_ms = 60_000;
    let corr = Signals { correction_p: Some(0.8), ..sure };
    assert!(f.run(&mut warm(80_000), Some(&corr), 3).unwrap().switched_provider);
}

#[test]
fn plan_steering_moves_off_claude_but_not_when_it_is_the_only_option() {
    let mut f = Fixture::new(&[Claude, Codex]);
    let s = sig(TaskType::Feature);
    assert_eq!(f.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.id, "claude-sonnet-5-5");
    f.plan = Some(0.89);
    assert_eq!(f.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.provider, Claude);
    f.plan = Some(0.90);
    let d = f.run(&mut RoutingState::default(), Some(&s), 20).unwrap();
    assert_eq!(d.spec.id, "gpt-6.1-terra");
    assert!(d.reason.contains("steered off claude"), "{}", d.reason);
    // At a boundary the steered chain also stops "keep the provider" from picking Claude.
    let d = f.run(&mut on("claude-sonnet-5-5"), Some(&Signals { task_confidence: 0.6, ..s.clone() }), 20).unwrap();
    assert_eq!(d.spec.provider, Codex);
    // Claude is the only usable provider: it still serves.
    let mut only = Fixture::new(&[Claude]);
    only.plan = Some(0.99);
    assert_eq!(only.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.id, "claude-sonnet-5-5");
}

#[test]
fn current_model_too_big_for_the_session_is_left() {
    let mut f = Fixture::new(&[Codex, Claude]);
    f.context_tokens = 600_000;
    let d = f.run(&mut on("gpt-6.1-terra"), Some(&sig(TaskType::Feature)), 3).unwrap();
    assert_eq!(d.spec.provider, Claude);
    assert!(d.switched_provider);
}

#[test]
fn short_follow_up_limit_matches_the_router() {
    let words = |n: usize| vec!["w"; n].join(" ");
    for n in 0..20 {
        assert_eq!(n <= SHORT_FOLLOW_UP_WORDS, jev_judgment::domain::security::is_short_follow_up(&words(n)), "{n}");
    }
}
