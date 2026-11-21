use super::*;
use crate::render::strip_ansi;

#[test]
fn overrides_change_accent_glyph_and_badge() {
    let (o, errors) = ThemeOverrides::parse("claude.accent=#112233; codex.glyph=*\n# comment\ngemini.badge=agy");
    assert!(errors.is_empty(), "{errors:?}");
    let th = ProviderTheme::new(ON, ColorDepth::TrueColor, Palette::Standard, o);
    assert_eq!(th.chip("claude"), "\x1b[38;2;17;34;51m◆ claude\x1b[0m");
    assert_eq!(strip_ansi(&th.chip("codex")), "* codex");
    assert_eq!(strip_ansi(&th.chip("gemini")), "✦ agy");
}

#[test]
fn bad_override_lines_are_reported_and_the_rest_apply() {
    let (o, errors) = ThemeOverrides::parse("claude.accent=red;codex.glyph=ab;grok.badge=;nothing;a.b=c;gemini.glyph=✦;deepseek.badge=ds");
    assert_eq!(errors.len(), 5);
    assert!(matches!(errors[0], OverrideError::BadColour(_)));
    assert!(matches!(errors[1], OverrideError::BadGlyph(_)));
    assert!(matches!(errors[2], OverrideError::BadBadge(_)));
    assert!(matches!(errors[3], OverrideError::Malformed(_)));
    assert!(matches!(errors[4], OverrideError::UnknownField(_)));
    assert!(errors.iter().all(|e| !e.to_string().is_empty()));
    let th = ProviderTheme::new(OFF, ColorDepth::Ansi16, Palette::Standard, o);
    assert_eq!(th.chip("deepseek"), "◉ ds");
    assert_eq!(th.chip("claude"), "◆ claude");
}

#[test]
fn merged_overrides_prefer_the_later_layer() {
    let (a, _) = ThemeOverrides::parse("claude.badge=one;claude.glyph=+");
    let (b, _) = ThemeOverrides::parse("claude.badge=two");
    let th = ProviderTheme::new(OFF, ColorDepth::Ansi16, Palette::Standard, a.merged(&b));
    assert_eq!(th.chip("claude"), "+ two");
}
