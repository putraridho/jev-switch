use super::*;

#[test]
fn a_hint_is_one_line_cut_to_the_width() {
    let long = format!("\u{2717} denied cd $(mktemp -d) && python3 - <<'EOF'\n{}\nEOF", "x = 1; ".repeat(60));
    let line = hint_line(&long, Style::off(), 80);
    assert!(!line.contains('\n') && crate::tui::render::display_width(&line) <= 80, "{line}");
    assert!(line.starts_with("  \u{2717} denied cd") && line.ends_with('\u{2026}'), "{line}");
    let auto = format!("\u{2713} auto-allowed cd $(mktemp -d) && python3 - <<'EOF'\n{}\nEOF", "x = 1; ".repeat(60));
    assert_eq!(hint_line(&auto, Style::off(), 80), "    \u{2713} auto-allowed");
    assert_eq!(hint_line("\u{2717} denied Bash", Style::off(), 80), "  \u{2717} denied Bash");
}


#[test]
fn gutter_is_off_unless_asked_for() {
    assert!(!gutter_setting(None));
    assert!(!gutter_setting(Some("0")));
    assert!(gutter_setting(Some("1")));
}
