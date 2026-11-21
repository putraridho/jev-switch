use super::*;

#[test]
fn native_events_use_each_providers_words_and_collapse() {
    let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
    let long = "a\nb\nc\nd\ne\nf";
    assert_eq!(th.native_event("claude", &NativeEvent::Thinking(long), 80), "  ◆ thinking\n    a\n    b\n    c\n    d\n    … 2 more");
    assert_eq!(th.native_event("codex", &NativeEvent::Thinking(long), 80), "  ▲ reasoning\n    a\n    b\n    … 4 more");
    assert_eq!(th.native_event("gemini", &NativeEvent::Thinking("x"), 80), "  ✦ thoughts\n    x");
    let files = vec!["a.rs".to_string(), "b.rs".to_string()];
    assert_eq!(th.native_event("codex", &NativeEvent::Patch { files: &files }, 80), "  ▲ apply_patch\n    + a.rs\n    + b.rs");
    assert_eq!(th.native_event("gemini", &NativeEvent::PlanStep { index: 2, total: 5, text: "write tests" }, 80), "  ✦ plan 2/5 write tests");
    for row in th.native_event("claude", &NativeEvent::Thinking(&"word ".repeat(40)), 30).lines() {
        assert!(display_width(row) < 30, "{row:?}");
    }
}
