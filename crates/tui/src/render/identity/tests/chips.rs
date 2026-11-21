use super::*;
use crate::render::strip_ansi;

#[test]
fn golden_chip_per_provider_and_depth() {
    let golden = [
        ("claude", ColorDepth::TrueColor, "\x1b[38;2;255;135;0m◆ claude\x1b[0m"),
        ("claude", ColorDepth::Ansi256, "\x1b[38;5;208m◆ claude\x1b[0m"),
        ("codex", ColorDepth::Ansi16, "\x1b[32m▲ codex\x1b[0m"),
        ("gemini", ColorDepth::Ansi256, "\x1b[38;5;134m✦ gemini\x1b[0m"),
        ("grok", ColorDepth::TrueColor, "\x1b[38;2;95;135;255m✕ grok\x1b[0m"),
        ("deepseek", ColorDepth::Ansi16, "\x1b[36m◉ deepseek\x1b[0m"),
    ];
    for (p, depth, want) in golden {
        assert_eq!(theme(ON, depth, Palette::Standard).chip(p), want, "{p} {depth:?}");
    }
    assert_eq!(theme(ON, ColorDepth::TrueColor, Palette::ColorBlind).chip("codex"), "\x1b[38;2;86;180;233m▲ codex\x1b[0m");
}

#[test]
fn a_disabled_style_is_plain_text_with_glyph_and_badge() {
    for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
        let th = theme(OFF, depth, Palette::Standard);
        assert_eq!(th.chip("codex"), "▲ codex");
        no_esc(&th.attribution_header(&[("claude", "m"), ("codex", "n")]));
        no_esc(&th.footer_chip("claude", &ChipFacts { plan: Some("max".into()), used: Some(0.9), cooldown: None, health: Health::Ok }));
    }
}

#[test]
fn the_answer_gets_a_gutter_and_a_header_naming_the_author() {
    let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
    assert_eq!(th.attributed_answer(&[("codex", "gpt-6.1-sol")], "one\ntwo\n"), "\n▌ ▲ codex · gpt-6.1-sol\n▌ one\n▌ two");
    assert_eq!(
        th.attributed_answer(&[("claude", "opus"), ("codex", "")], "x"),
        "\n▌ ◆ claude · opus  ⇄  ▲ codex\n▌ x"
    );
    assert_eq!(th.attributed_answer(&[], "x\n"), "\nx");
    let coloured = theme(ON, ColorDepth::Ansi16, Palette::Standard).attributed_answer(&[("codex", "m")], "one");
    assert_eq!(strip_ansi(&coloured), "\n▌ ▲ codex · m\n▌ one");
    assert!(coloured.contains("\x1b[32m▌"));
}

#[test]
fn footer_chip_shows_plan_gauge_and_cooldown() {
    let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
    let facts = ChipFacts { plan: Some("max_5x".into()), used: Some(0.6), cooldown: None, health: Health::Ok };
    assert_eq!(th.footer_chip("claude", &facts), "◆ claude max_5x ▰▰▰▱▱ 60%");
    let hot = ChipFacts { used: Some(0.97), cooldown: Some("2h 10m".into()), health: Health::CoolingDown, ..facts.clone() };
    assert_eq!(th.footer_chip("claude", &hot), "◆ claude max_5x ▰▰▰▰▰ 97% ⏳ 2h 10m");
    let down = ChipFacts { plan: None, used: None, cooldown: None, health: Health::Unavailable };
    assert_eq!(th.footer_chip("gemini", &down), "✦ gemini ✗ unavailable");
    let styled = theme(ON, ColorDepth::Ansi16, Palette::Standard).footer_chip("claude", &hot);
    assert!(styled.contains("\x1b[31m▰▰▰▰▰ 97%"), "critical is red: {styled:?}");
}
