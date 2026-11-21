use super::*;
use crate::tui::render::Style;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::Context;

#[test]
fn the_hint_is_dim_only_when_styled() {
    let h = JevHelper::new(Style::off(), SharedMode::default());
    assert_eq!(h.highlight_hint("el"), "el");
    assert_eq!(JevHelper::new(Style::on(), SharedMode::default()).highlight_hint("el"), "\x1b[2mel\x1b[0m");
}

#[test]
fn an_empty_line_hints_the_mode_and_a_typed_one_does_not() {
    use crate::approval::domain::mode::Mode;
    let h = JevHelper::new(Style::off(), SharedMode::new(Mode::Auto));
    let history = DefaultHistory::new();
    let ctx = Context::new(&history);
    assert_eq!(h.hint("", 0, &ctx).as_deref(), Some("[auto mode \u{b7} shift-tab to change]"));
    assert_eq!(h.hint("hello", 5, &ctx), None);
}

#[test]
fn the_prompt_is_coloured_only_when_it_is_ours_and_colour_is_on() {
    let mut h = JevHelper::new(Style::on(), SharedMode::default());
    h.set_prompt("› ", "\x1b[36m›\x1b[0m ");
    assert_eq!(h.highlight_prompt("› ", true), "\x1b[36m›\x1b[0m ");
    assert_eq!(h.highlight_prompt("› ", false), "› ", "an incremental-search prompt stays plain");
    assert_eq!(h.highlight_prompt("(reverse-i-search)`': ", true), "(reverse-i-search)`': ");
    let mut off = JevHelper::new(Style::off(), SharedMode::default());
    off.set_prompt("› ", "x");
    assert_eq!(off.highlight_prompt("› ", true), "› ");
}

#[test]
fn yes_means_y_or_yes_and_nothing_else() {
    for yes in ["y", "Y", "yes", " YES "] {
        assert!(is_yes(yes), "{yes:?}");
    }
    for no in ["", "n", "no", "yep", "ye", "1"] {
        assert!(!is_yes(no), "{no:?}");
    }
}

#[test]
fn a_failure_of_the_editor_is_described_in_words() {
    let e = ReadlineError::Io(std::io::Error::other("boom"));
    assert_eq!(describe(&e), "the terminal failed: boom");
}
