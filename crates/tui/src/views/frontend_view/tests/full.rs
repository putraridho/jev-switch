use super::*;

pub(super) fn full_app() -> App {
    let mut a = app();
    a.fullscreen = true;
    a.git = Some("main ±3".into());
    a.providers = vec![crate::app::screen_state::ProviderChip { name: "codex".into(), used: Some(71), cooling: None, via: None, credit: None }];
    a.show("one\ntwo\nthree");
    a
}

#[test]
fn full_screen_is_the_header_on_top_the_chat_full_width_and_the_box_at_the_bottom() {
    let a = full_app();
    let (f, pane) = render_full(&a, 140, 40, Instant::now(), OFF);
    assert_eq!(f.rows.len(), 40);
    assert!(f.rows.iter().all(|r| display_width(r) < 140), "no row reaches the last column");
    assert!(f.rows[0].starts_with(" main ±3 /nonexistent") && f.rows[0].ends_with("⇄ auto · codex 71%"), "{}", f.rows[0]);
    // The chat's newest lines sit just above the box, nothing beside them.
    assert_eq!(f.rows[pane], "three");
    assert!(!f.rows.join("\n").contains("Router"));
    // The cursor is in the box, under the chat.
    let (row, col) = f.cursor.unwrap_or_default();
    assert_eq!(row, pane + 2);
    assert_eq!(col, 4);
    assert!(f.rows[row].contains("Ask anything"), "{}", f.rows[row]);
    assert!(f.rows[39].starts_with("  Enter:send"), "{}", f.rows[39]);
}

#[test]
fn the_answer_being_written_is_drawn_in_the_chat_until_it_is_committed() {
    let mut a = full_app();
    a.live_answer = Some("Hel".into());
    let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
    assert!(f.rows.join("\n").contains("Hel"));
    // Scrolled back, the preview stays out of the way.
    a.show(&(1..=30).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"));
    a.transcript.scroll_by(5, 10);
    let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
    assert!(!f.rows.join("\n").contains("Hel"));
    a.transcript.to_end();
    a.live_answer = None;
    let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
    assert!(!f.rows.join("\n").contains("Hel"));
}

#[test]
fn full_screen_survives_a_terminal_shorter_than_the_live_area() {
    let mut a = full_app();
    a.on_front(FrontMsg::TurnStarted);
    for height in [1, 3, 5, 8] {
        let (f, _) = render_full(&a, 120, height, Instant::now(), OFF);
        assert_eq!(f.rows.len(), height, "height {height}");
        if let Some((r, _)) = f.cursor {
            assert!(r < height);
        }
    }
}

#[test]
fn page_up_scrolls_the_chat_back_and_new_output_leaves_the_view_alone() {
    let mut a = full_app();
    a.show(&(0..60).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"));
    let (f, pane) = render_full(&a, 140, 30, Instant::now(), OFF);
    a.pane_rows = pane;
    assert!(f.rows[pane].contains("line 59"));
    a.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), Instant::now());
    let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
    assert!(!f.rows[pane].contains("line 59"), "{}", f.rows[pane]);
    a.show("a new line");
    let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
    assert!(!f.rows.join("\n").contains("a new line"), "the view stays where it was");
    a.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL), Instant::now());
    let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
    assert!(f.rows[pane].contains("a new line"));
}
