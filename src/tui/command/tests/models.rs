//! Model pins, routing, orchestration and thinking toggles.

use super::*;
use crate::backend::ProviderId;

#[test]
fn model_pins() {
    assert_eq!(parse("/model"), Command::ShowModel);
    assert_eq!(parse("/model auto"), Command::SetPin(Pin::Auto));
    assert_eq!(parse("/model codex"), Command::SetPin(Pin::Provider(ProviderId::Codex)));
    assert_eq!(
        parse("/model sonnet"),
        Command::SetPin(Pin::Model(ProviderId::Claude, "claude-sonnet-5-5".into()))
    );
    assert_eq!(
        parse("/model codex:gpt-6.1-sol"),
        Command::SetPin(Pin::Model(ProviderId::Codex, "gpt-6.1-sol".into()))
    );
    assert!(matches!(parse("/model nonsense"), Command::Invalid(m) if m.contains("unknown provider")));
    assert!(matches!(parse("/model codex:nope"), Command::Invalid(m) if m.contains("unknown model")));
    assert!(matches!(parse("/model a b"), Command::Invalid(_)));
}

#[test]
fn pins_print_the_way_they_are_typed() {
    for arg in ["auto", "codex", "claude:claude-opus-5-5"] {
        assert_eq!(Pin::parse(arg).unwrap().to_string(), arg);
    }
}

#[test]
fn routing_toggle() {
    assert_eq!(parse("/provider auto"), Command::SetRouting(true));
    assert_eq!(parse("/provider off"), Command::SetRouting(false));
    assert!(matches!(parse("/provider"), Command::Invalid(_)));
    assert!(matches!(parse("/provider maybe"), Command::Invalid(_)));
}

#[test]
fn orchestrate_toggle() {
    assert_eq!(parse("/orchestrate on"), Command::SetOrchestrate(true));
    assert_eq!(parse("/orchestrate off"), Command::SetOrchestrate(false));
    assert_eq!(parse("/orchestrate"), Command::ShowOrchestrate);
    assert_eq!(parse("/mode"), Command::ShowMode);
    assert_eq!(parse("/mode auto"), Command::SetMode(Mode::Auto));
    assert_eq!(parse("/mode acceptEdits"), Command::SetMode(Mode::Edit));
    assert!(matches!(parse("/mode yolo"), Command::Invalid(m) if m.contains("ask, edit, plan or auto")));
    assert!(matches!(parse("/mode auto plan"), Command::Invalid(_)));
    assert!(matches!(parse("/orchestrate maybe"), Command::Invalid(m) if m.contains("on|off")));
    assert!(matches!(parse("/orchestrate on now"), Command::Invalid(_)));
}

#[test]
fn thinking_toggle() {
    assert_eq!(parse("/thinking on"), Command::SetThinking(true));
    assert_eq!(parse("/thinking off"), Command::SetThinking(false));
    assert_eq!(parse("/thinking"), Command::ShowThinking);
    assert!(matches!(parse("/thinking maybe"), Command::Invalid(m) if m.contains("on|off")));
    assert!(matches!(parse("/thinking on now"), Command::Invalid(_)));
}
