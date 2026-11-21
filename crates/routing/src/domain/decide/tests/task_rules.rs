//! The task type: the hard security rule and the no-Jev fallback.

use super::*;

#[test]
fn security_rule_beats_jevs_task_type() {
    let strong = Signals { security_keywords: Security::Strong, ..sig(TaskType::Docs) };
    let by_p = Signals { security_p: 0.9, ..sig(TaskType::Docs) };
    let weak_edit = Signals { security_keywords: Security::Weak, edit_p: 0.6, ..sig(TaskType::Docs) };
    let weak_no_edit = Signals { security_keywords: Security::Weak, edit_p: 0.1, ..sig(TaskType::Docs) };
    let f = Fixture::new(&PROVIDERS);
    for (s, security) in [(strong, true), (by_p, true), (weak_edit, true), (weak_no_edit, false)] {
        let d = f.run(&mut RoutingState::default(), Some(&s), NEW_TASK_WORDS).unwrap();
        assert_eq!(d.task == TaskType::Security, security, "{s:?}");
        if security {
            assert!(d.reason.contains("security rule"), "{}", d.reason);
            assert!(allowed_for(TaskType::Security, d.spec));
        }
    }
}

#[test]
fn security_with_only_deepseek_or_grok_is_nothing_usable() {
    let s = Signals { security_keywords: Security::Strong, ..sig(TaskType::Feature) };
    for provs in [vec![DeepSeek], vec![Grok], vec![DeepSeek, Grok]] {
        let f = Fixture::new(&provs);
        assert_eq!(
            f.run(&mut RoutingState::default(), Some(&s), 20).unwrap_err(),
            NoRoute::NothingUsable { task: TaskType::Security }
        );
    }
}

#[test]
fn no_signals_stays_on_the_current_model_if_usable() {
    let f = Fixture::new(&PROVIDERS);
    let mut st = on("deepseek-flash");
    let d = f.run(&mut st, None, 3).unwrap();
    assert_eq!((d.spec.id, d.switched_provider, d.task), ("deepseek-flash", false, TaskType::Unclear));
    // No current model: the Unclear chain's first choice.
    let d = f.run(&mut RoutingState::default(), None, 3).unwrap();
    assert_eq!(d.spec.id, "claude-sonnet-5-5");
    // The current model is gone: move to a usable one.
    let f = Fixture::new(&[Codex]);
    let d = f.run(&mut on("deepseek-flash"), None, 3).unwrap();
    assert_eq!(d.spec.id, "gpt-6.1-terra");
    assert!(d.switched_provider);
}

#[test]
fn dropped_tier_comes_from_the_pick() {
    // Sonnet is what explaining needs, even though Grok 4.7 heads the chain.
    let f = Fixture::new(&[Claude]);
    assert!(!f.run(&mut RoutingState::default(), Some(&sig(TaskType::Explain)), 20).unwrap().dropped_tier);
    // Only a light model left for an explanation: a real drop.
    let f = Fixture::new(&[Gemini]);
    assert!(f.run(&mut RoutingState::default(), Some(&sig(TaskType::Explain)), 20).unwrap().dropped_tier);
}
