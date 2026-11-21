use super::*;

#[test]
fn enter_sends_and_shift_or_alt_enter_or_ctrl_j_or_backslash_adds_a_line() {
    let mut a = app();
    typed(&mut a, "one");
    a.on_key(with(KeyCode::Enter, KeyModifiers::SHIFT), Instant::now());
    typed(&mut a, "two");
    a.on_key(with(KeyCode::Enter, KeyModifiers::ALT), Instant::now());
    a.on_key(with(KeyCode::Char('j'), KeyModifiers::CONTROL), Instant::now());
    typed(&mut a, "three\\");
    a.on_key(key(KeyCode::Enter), Instant::now());
    typed(&mut a, "four");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("one\ntwo\n\nthree\nfour"));
    assert!(a.editor.is_empty());
}

#[test]
fn a_message_the_model_took_into_the_turn_leaves_the_queue() {
    let mut a = app();
    a.on_front(FrontMsg::TurnStarted);
    a.queue.extend(["first".to_string(), "second".to_string()]);
    a.on_front(FrontMsg::Steered("first".into()));
    assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["second".to_string()]);
    // A message nobody queued changes nothing.
    a.on_front(FrontMsg::Steered("other".into()));
    assert_eq!(a.queue.len(), 1);
}

#[test]
fn typing_during_a_turn_queues_and_esc_stops_sending_the_text_first() {
    let mut a = app();
    a.on_front(FrontMsg::TurnStarted);
    typed(&mut a, "later");
    // Queued, and offered to the running turn.
    assert!(matches!(a.on_key(key(KeyCode::Enter), Instant::now()).as_slice(), [Action::Steer(t)] if t == "later"));
    typed(&mut a, "now");
    let out = a.on_key(key(KeyCode::Esc), Instant::now());
    assert!(matches!(out.as_slice(), [Action::Stop]));
    assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["now".to_string(), "later".to_string()]);
}

#[test]
fn ctrl_c_stops_a_turn_and_drops_the_queue_and_twice_on_an_empty_box_exits() {
    let mut a = app();
    let ctrl_c = with(KeyCode::Char('c'), KeyModifiers::CONTROL);
    a.on_front(FrontMsg::TurnStarted);
    a.queue.push_back("queued".into());
    let out = a.on_key(ctrl_c, Instant::now());
    assert!(matches!(out.as_slice(), [Action::Stop, Action::Print(_)]));
    assert!(a.queue.is_empty());
    a.on_front(FrontMsg::TurnEnded);

    typed(&mut a, "draft");
    assert!(a.on_key(ctrl_c, Instant::now()).is_empty());
    assert!(a.editor.is_empty(), "the first Ctrl-C clears the text");
    let t = Instant::now();
    assert!(a.on_key(ctrl_c, t).is_empty());
    assert_eq!(a.notice.as_ref().map(|n| n.0.as_str()), Some("Press Ctrl-C again to exit"));
    assert!(matches!(a.on_key(ctrl_c, t + Duration::from_millis(500)).as_slice(), [Action::Quit]));
}

#[test]
fn up_browses_history_and_down_comes_back() {
    let mut a = app();
    typed(&mut a, "draft");
    a.on_key(key(KeyCode::Up), Instant::now());
    assert_eq!(a.editor.text(), "older message");
    a.on_key(key(KeyCode::Down), Instant::now());
    assert_eq!(a.editor.text(), "draft");
}

#[test]
fn up_takes_the_last_queued_message_back_into_the_box() {
    let mut a = app();
    a.on_front(FrontMsg::TurnStarted);
    typed(&mut a, "first");
    a.on_key(key(KeyCode::Enter), Instant::now());
    typed(&mut a, "second");
    a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["first".to_string(), "second".to_string()]);

    // The copy already offered to the turn is withdrawn, or the turn would send it AND the edit.
    let out = a.on_key(key(KeyCode::Up), Instant::now());
    assert!(matches!(out.as_slice(), [Action::Unsteer(t)] if t == "second"), "{out:?}");
    assert_eq!(a.editor.text(), "second", "↑ pops the newest queued message");
    assert_eq!(a.queue.iter().cloned().collect::<Vec<_>>(), vec!["first".to_string()]);

    // The box holds that one now; clearing it lets ↑ take the other.
    a.on_key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), Instant::now());
    a.on_key(key(KeyCode::Up), Instant::now());
    assert_eq!(a.editor.text(), "first");
    assert!(a.queue.is_empty());

    // With nothing queued, ↑ is history again (the messages sent above are in it).
    a.on_key(with(KeyCode::Char('u'), KeyModifiers::CONTROL), Instant::now());
    a.on_key(key(KeyCode::Up), Instant::now());
    assert_eq!(a.editor.text(), "second");
}

#[test]
fn a_draft_in_the_box_is_not_thrown_away_for_a_queued_message() {
    let mut a = app();
    a.on_front(FrontMsg::TurnStarted);
    a.queue.push_back("queued".into());
    typed(&mut a, "draft");
    a.on_key(key(KeyCode::Up), Instant::now());
    assert_eq!(a.editor.text(), "older message", "↑ is history while the box has a draft");
    assert_eq!(a.queue.len(), 1);
}

#[test]
fn shift_tab_cycles_the_mode() {
    let mut a = app();
    let out = a.on_key(key(KeyCode::BackTab), Instant::now());
    assert!(matches!(out.as_slice(), [Action::Mode(Mode::Plan)]), "edit → plan");
}

#[test]
fn a_question_mark_on_an_empty_box_opens_the_cheat_sheet_but_is_text_elsewhere() {
    let mut a = app();
    assert_eq!(submitted(&a.on_key(key(KeyCode::Char('?')), Instant::now())), Some("/keys"));
    let mut b = app();
    typed(&mut b, "why?");
    assert_eq!(b.editor.text(), "why?");
}

#[test]
fn vim_mode_moves_and_edits_in_normal_mode_and_sends_on_enter() {
    let mut a = app();
    a.vim = Some(Vim::default());
    typed(&mut a, "hello");
    a.on_key(key(KeyCode::Esc), Instant::now());
    typed(&mut a, "0x");
    assert_eq!(a.editor.text(), "ello");
    typed(&mut a, "A!");
    assert_eq!(a.editor.text(), "ello!");
    a.on_key(key(KeyCode::Esc), Instant::now());
    assert_eq!(submitted(&a.on_key(key(KeyCode::Enter), Instant::now())), Some("ello!"));
}

#[test]
fn a_user_binding_beats_the_default_and_runs_its_action() {
    let mut a = app();
    a.keymap = Keymap::parse("ctrl-o = clear\nf2 = quit\n").0;
    let out = a.on_key(with(KeyCode::Char('o'), KeyModifiers::CONTROL), Instant::now());
    assert!(matches!(out.as_slice(), [Action::ClearScreen]));
    assert!(matches!(a.on_key(key(KeyCode::F(2)), Instant::now()).as_slice(), [Action::Quit]));
}

#[test]
fn paste_keeps_its_lines_in_one_message() {
    let mut a = app();
    a.on_paste("line one\nline two");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("line one\nline two"));
}
