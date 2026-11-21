//! `/model` pins: honored exactly or reported, never swapped.

use super::*;

#[test]
fn pin_is_honored_or_an_error_never_swapped() {
    // (pin, providers, task, expected model or None for an error)
    let cases: Vec<(Pin, Vec<ProviderId>, TaskType, Option<&str>)> = vec![
        (Pin::Model(DeepSeek, "deepseek-flash".into()), vec![Claude, DeepSeek], TaskType::HardDebug, Some("deepseek-flash")),
        (Pin::Model(DeepSeek, "deepseek-flash".into()), vec![Claude], TaskType::Explain, None),
        (Pin::Model(Claude, "nope".into()), vec![Claude], TaskType::Explain, None),
        (Pin::Provider(Codex), vec![Claude, Codex], TaskType::Feature, Some("gpt-6.1-terra")),
        (Pin::Provider(Codex), vec![Claude], TaskType::Feature, None),
        // Pinned to a vendor the Security rule bars: reported, not rerouted.
        (Pin::Provider(Grok), vec![Claude, Grok], TaskType::Security, None),
    ];
    for (pin, provs, task, want) in cases {
        let mut f = Fixture::new(&provs);
        f.pin = pin.clone();
        let got = f.run(&mut RoutingState::default(), Some(&sig(task)), NEW_TASK_WORDS);
        match want {
            Some(id) => assert_eq!(got.unwrap().spec.id, id, "{pin:?}"),
            None => assert!(matches!(got, Err(NoRoute::PinUnusable { .. })), "{pin:?}: {got:?}"),
        }
    }
}

#[test]
fn pin_respects_billing_and_context() {
    let mut f = Fixture::new(&[Claude]);
    f.pin = Pin::Model(Claude, "claude-fable-5-2".into());
    f.allow.credits = false;
    let err = f.run(&mut RoutingState::default(), Some(&sig(TaskType::Feature)), 5).unwrap_err();
    assert!(err.to_string().contains("billing"), "{err}");
    f.allow.credits = true;
    assert!(f.run(&mut RoutingState::default(), None, 5).is_ok());
    let mut f = Fixture::new(&[Codex]);
    f.pin = Pin::Model(Codex, "gpt-6.1-sol".into());
    f.context_tokens = 600_000;
    assert!(matches!(f.run(&mut RoutingState::default(), None, 5), Err(NoRoute::PinUnusable { .. })));
}
