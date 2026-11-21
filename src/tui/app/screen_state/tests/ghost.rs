use super::*;

#[test]
fn right_takes_the_suggested_next_prompt_and_sending_clears_it() {
    let mut a = app();
    a.suggestion = Some("commit this and tag v1.1.0".into());
    assert_eq!(a.ghost().as_deref(), Some("commit this and tag v1.1.0"));
    a.on_key(key(KeyCode::Right), Instant::now());
    assert_eq!(a.editor.text(), "commit this and tag v1.1.0");
    assert_eq!(a.ghost(), None, "nothing left to add");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("commit this and tag v1.1.0"));
    assert_eq!((a.suggestion.as_deref(), a.ghost()), (None, None));
}

#[test]
fn the_suggestion_waits_for_an_empty_idle_box_and_typing_something_else_hides_it() {
    let mut a = app();
    a.suggestion = Some("push the branch".into());
    a.working = Some(Working { label: "thinking".into(), activity: None, started: Instant::now() });
    assert_eq!(a.ghost(), None, "not while a turn runs");
    a.working = None;
    typed(&mut a, "zz");
    assert_eq!(a.ghost(), None, "typed text that matches nothing has no ghost");
    a.on_key(key(KeyCode::Backspace), Instant::now());
    a.on_key(key(KeyCode::Backspace), Instant::now());
    assert_eq!(a.ghost().as_deref(), Some("push the branch"), "an empty box shows it again");
    // A typed message replaces the offer once sent.
    typed(&mut a, "something else");
    a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(a.suggestion, None);
}

#[test]
fn typed_text_ghosts_the_newest_matching_message_and_right_takes_it_only_at_the_end() {
    let mut a = app();
    typed(&mut a, "older");
    assert_eq!(a.ghost().as_deref(), Some(" message"));
    a.on_key(key(KeyCode::Left), Instant::now());
    assert_eq!(a.ghost(), None, "not with the cursor inside the text");
    a.on_key(key(KeyCode::Right), Instant::now());
    assert_eq!((a.editor.text(), a.editor.cursor()), ("older", 5), "Right moves the cursor when there is no ghost");
    a.on_key(key(KeyCode::Right), Instant::now());
    assert_eq!(a.editor.text(), "older message");
}
