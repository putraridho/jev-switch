//! Which models a turn may use, and the `/providers` lines.

use super::ready;
use crate::domain::availability::render::render;
use crate::domain::availability::usable::usable_models;
use crate::domain::availability::{avail, Availability, State};
use jev_backend::ProviderId;

fn catalog(p: ProviderId) -> Vec<String> {
    match p {
        ProviderId::Claude => vec!["opus".into(), "sonnet".into()],
        ProviderId::Codex => vec!["codex-all".into()],
        _ => vec![],
    }
}

#[test]
fn usable_models_uses_listed_models_else_the_catalog_and_skips_unready() {
    let list = vec![
        ready(ProviderId::Claude, None),
        ready(ProviderId::Codex, Some(vec!["gpt-a"])),
        avail(ProviderId::Gemini, State::NotLoggedIn, "x"),
        avail(ProviderId::DeepSeek, State::Unknown, "x"),
    ];
    assert_eq!(
        usable_models(&list, &catalog),
        vec![
            (ProviderId::Claude, "opus".to_string()),
            (ProviderId::Claude, "sonnet".to_string()),
            (ProviderId::Codex, "gpt-a".to_string()),
        ]
    );
    assert!(usable_models(&[], &catalog).is_empty());
}

// ---- render

#[test]
fn render_shows_state_plan_models_detail_and_unverified() {
    let list = vec![
        Availability { plan: Some("max_5x".into()), ..ready(ProviderId::Claude, None) },
        ready(ProviderId::Codex, Some(vec!["a", "b"])),
        Availability { verified: false, ..avail(ProviderId::Grok, State::NotLoggedIn, "sign in") },
    ];
    let out = render(&list);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("claude") && lines[0].contains("ready") && lines[0].contains("plan: max_5x"));
    assert!(lines[0].contains("models: all") && !lines[0].contains("unverified"));
    assert!(lines[1].contains("models: a, b") && lines[1].contains("plan: -"));
    assert!(lines[2].contains("not logged in") && lines[2].contains("models: -") && lines[2].ends_with("(unverified)"));
}
