use super::*;
use crate::tui::render::Style;
use super::super::history::*;

#[test]
fn history_is_a_private_file_that_remembers_lines_across_editors() {
    let dir = temp("history");
    let path = history_path(&dir);
    let mut first = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
    remember(&mut first, "fix the bug", Some(&path));
    remember(&mut first, "   ", Some(&path));
    remember(&mut first, "fix the bug", Some(&path));
    remember(&mut first, "/model codex", Some(&path));
    assert_eq!(history_len(&first), 2, "blank lines and consecutive duplicates are not kept");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let second = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
    let last: Vec<String> = second.history().iter().cloned().collect();
    assert_eq!(last, ["fix the bug", "/model codex"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn multi_line_messages_survive_in_history() {
    let dir = temp("multiline");
    let path = history_path(&dir);
    let mut first = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
    remember(&mut first, "line one\nline two", Some(&path));
    let second = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
    assert_eq!(second.history().iter().next().map(String::as_str), Some("line one\nline two"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_history_is_capped() {
    let dir = temp("cap");
    let path = history_path(&dir);
    let mut e = build_editor(Style::off(), Some(&path), SharedMode::default()).unwrap();
    for i in 0..HISTORY_MAX + 10 {
        remember(&mut e, &format!("message {i}"), Some(&path));
    }
    assert_eq!(history_len(&e), HISTORY_MAX);
    let _ = std::fs::remove_dir_all(dir);
}
