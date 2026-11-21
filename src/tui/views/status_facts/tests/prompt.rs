use super::*;
use crate::routing::domain::decide::Pin;
use crate::tui::render as uifmt;
use crate::tui::render::Style;
use super::super::prompt::{plain_prompt, prompt_label, styled_prompt};

#[test]
fn the_prompt_shows_the_pin_or_routing_off() {
    use crate::backend::ProviderId::Codex;
    assert_eq!(prompt_label(&Pin::Auto, true), None);
    assert_eq!(prompt_label(&Pin::Auto, false).as_deref(), Some("claude only"));
    assert_eq!(prompt_label(&Pin::Provider(Codex), true).as_deref(), Some("codex"));
    assert_eq!(prompt_label(&Pin::Model(Claude, "claude-opus-5-5".into()), true).as_deref(), Some("claude:claude-opus-5-5"));
    assert_eq!(plain_prompt(None), "› ");
    assert_eq!(plain_prompt(Some("codex")), "[codex] › ");
}

#[test]
fn the_styled_prompt_is_the_plain_one_with_colour() {
    for label in [None, Some("codex"), Some("claude:claude-opus-5-5"), Some("claude only")] {
        assert_eq!(uifmt::strip_ansi(&styled_prompt(label, Style::on())), plain_prompt(label));
        assert_eq!(styled_prompt(label, Style::off()), plain_prompt(label));
    }
    assert!(styled_prompt(Some("codex"), Style::on()).contains("\x1b[32m"), "codex is green");
}
