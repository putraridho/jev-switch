use super::*;

#[test]
fn a_slash_opens_the_menu_and_enter_runs_a_complete_command() {
    let mut a = app();
    typed(&mut a, "/");
    assert_eq!(a.menu.as_ref().map(|m| m.items[0].replacement.as_str()), Some("/model"));
    typed(&mut a, "hel");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("/help"));
}

#[test]
fn a_command_with_arguments_takes_a_space_and_shows_its_choices() {
    let mut a = app();
    typed(&mut a, "/mod");
    a.on_key(key(KeyCode::Down), Instant::now());
    let out = a.on_key(key(KeyCode::Tab), Instant::now());
    assert!(out.is_empty());
    assert_eq!(a.editor.text(), "/mode ");
    let choices: Vec<_> = a.menu.as_ref().map(|m| m.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default();
    assert_eq!(choices, vec!["ask", "edit", "plan", "auto"]);
    a.on_key(key(KeyCode::Down), Instant::now());
    a.on_key(key(KeyCode::Down), Instant::now());
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("/mode plan"));
}

#[test]
fn esc_closes_the_menu_until_the_text_changes() {
    let mut a = app();
    typed(&mut a, "/c");
    assert!(a.menu.is_some());
    a.on_key(key(KeyCode::Esc), Instant::now());
    assert!(a.menu.is_none());
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("/c"), "with the menu closed Enter sends the text as typed");
    typed(&mut a, "/c");
    assert!(a.menu.is_some(), "new text, new menu");
}

#[test]
fn enter_never_swaps_in_a_command_found_by_scattered_letters() {
    let mut a = app();
    typed(&mut a, "/provders");
    assert!(a.menu.is_some(), "the menu offers /providers");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("/provders"), "sent as typed, to be told what was meant");
    typed(&mut a, "/provders");
    a.on_key(key(KeyCode::Down), Instant::now());
    a.on_key(key(KeyCode::Up), Instant::now());
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), None, "picked on purpose: put in, takes arguments");
    assert_eq!(a.editor.text(), "/providers ");
}

#[test]
fn enter_on_a_complete_command_with_optional_arguments_runs_it() {
    let mut a = app();
    typed(&mut a, "/permissions");
    let out = a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(submitted(&out), Some("/permissions"));
}
