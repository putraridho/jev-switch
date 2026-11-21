use super::*;

#[test]
fn the_idle_screen_is_the_header_the_box_with_the_mode_in_its_edge_and_the_keys() {
    let a = app();
    let f = render(&a, 70, Instant::now(), OFF);
    assert!(f.rows[0].starts_with(" /nonexistent") && f.rows[0].ends_with("⇄ auto · no provider ready · /doctor"), "the header: {}", f.rows[0]);
    assert_eq!(f.rows[1], format!("╭{}╮", "─".repeat(67)));
    assert_eq!(f.rows[2], format!("│ ❯ Ask anything · / for commands · @ for files{}│", " ".repeat(21)));
    assert!(f.rows[3].starts_with("╰──") && f.rows[3].ends_with(" auto · edit mode ─╯"), "the mode in the box's edge: {}", f.rows[3]);
    assert_eq!(display_width(&f.rows[3]), 69);
    assert_eq!(f.rows[4], "  Enter:send │ Shift+Enter:newline │ Shift+Tab:mode │ /:commands", "a hint that does not fit is left off whole");
    assert_eq!(render(&a, 90, Instant::now(), OFF).rows[4], "  Enter:send │ Shift+Enter:newline │ Shift+Tab:mode │ /:commands │ @:files");
    assert_eq!(f.rows.len(), 5, "nothing else is boxed or listed: {:?}", f.rows);
    assert_eq!(f.cursor, Some((2, 4)));
}

#[test]
fn a_suggestion_takes_the_placeholders_place_and_ghost_text_follows_the_typed_text() {
    let mut a = app();
    a.suggestion = Some("commit this and tag v1.1.0".into());
    let f = render(&a, 70, Instant::now(), OFF);
    assert!(f.rows[2].starts_with("│ ❯ commit this and tag v1.1.0 "), "{}", f.rows[2]);
    assert!(f.rows[3].starts_with("╰─ → to use ") && f.rows[3].ends_with("edit mode ─╯"), "{}", f.rows[3]);
    assert_eq!(f.cursor, Some((2, 4)), "the cursor sits before the suggestion");

    let mut typing = App::new(Path::new("/nonexistent").into(), History::from_entries(&["fix the parser"]), crate::approval::domain::mode::Mode::Edit, "auto".into(), Vec::new());
    typed(&mut typing, "fix t");
    let f = render(&typing, 70, Instant::now(), OFF);
    assert!(f.rows[2].starts_with("│ ❯ fix the parser "), "{}", f.rows[2]);
    assert_eq!(f.cursor, Some((2, 9)), "the cursor stays after what was typed");
}

#[test]
fn typing_a_slash_opens_the_menu_above_the_box_with_names_and_descriptions() {
    let mut a = app();
    typed(&mut a, "/");
    let f = render(&a, 80, Instant::now(), OFF);
    let s = f.rows.join("\n");
    let box_top = f.rows.iter().position(|r| r.starts_with('╭')).unwrap_or(0);
    let selected = f.rows.iter().position(|r| r.starts_with(" ❯ /model  ")).unwrap_or(usize::MAX);
    assert!(selected < box_top, "the menu sits above the box, the first command selected: {s}");
    assert!(f.rows[1].starts_with("  /model [target]") && f.rows[1].ends_with(&format!("1/{}", crate::tui::render::COMMANDS.len())), "the heading row: {}", f.rows[1]);
    assert!(s.contains("Pin a model or provider, or `auto` to let the router choose"), "names only, so the description fits at 80 columns: {s}");
    assert!(!s.contains("/effort ["), "arguments stay out of the list: {s}");
    assert_eq!(f.rows.iter().filter(|r| r.starts_with("   /") || r.starts_with(" ❯ /")).count(), MENU_ROWS);
    assert!(f.rows.last().is_some_and(|r| r == "  ↑↓:select │ Tab:complete │ Enter:run │ Esc:close"), "the keys say what they do in the menu: {s}");
    a.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Instant::now());
    assert!(screen(&a, 80).contains("  /provider auto|off"), "the heading follows the selection");
    // With colour the selected row is raised the width of the row.
    let on = render(&a, 80, Instant::now(), ON);
    assert!(on.rows.iter().any(|r| r.contains("\x1b[48;5;236") && strip_ansi(r).starts_with(" ❯ /provider")), "{:?}", on.rows);
}

#[test]
fn a_long_message_wraps_inside_the_box_and_the_cursor_follows() {
    let mut a = app();
    typed(&mut a, &"word ".repeat(20));
    let f = render(&a, 40, Instant::now(), OFF);
    let inside: Vec<_> = f.rows.iter().filter(|r| r.starts_with('│')).collect();
    assert!(inside.len() >= 3, "{:?}", f.rows);
    let (row, col) = f.cursor.unwrap_or_default();
    assert!(row >= 4 && col >= 4, "{:?}", f.cursor);
}

#[test]
fn a_notice_takes_the_place_of_the_keys_and_the_mode_is_named_not_only_coloured() {
    let mut a = app();
    a.notice = Some(("Press Ctrl-C again to exit".into(), Instant::now()));
    let s = screen(&a, 80);
    assert_eq!(s.lines().last(), Some("  Press Ctrl-C again to exit"), "{s}");
    assert!(s.contains("edit mode ─╯"), "{s}");
    a.mode = crate::approval::domain::mode::Mode::Plan;
    assert!(screen(&a, 80).contains(" auto · plan mode ─╯"));
}
