use super::*;
use crossterm::event::KeyEvent;
use jev_approval::domain::mode::Mode;

#[test]
fn up_does_not_withdraw_a_message_already_taken_by_the_model() {
    let mut app = App::new(PathBuf::from("/nonexistent"), History::default(), Mode::Edit, "auto".into(), Vec::new());
    app.on_front(FrontMsg::TurnStarted);
    app.queue.extend(["still waiting".to_string(), "already taken".to_string()]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(FrontMsg::Steered("already taken".into())).unwrap();

    let actions = key_after_updates(&mut app, &mut rx, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), Instant::now());

    assert_eq!(app.editor.text(), "still waiting");
    assert!(app.queue.is_empty());
    assert!(matches!(actions.as_slice(), [Action::Unsteer(text)] if text == "still waiting"));
}

#[test]
fn ctrl_c_withdraws_each_waiting_message_before_stopping() {
    let mut app = App::new(PathBuf::from("/nonexistent"), History::default(), Mode::Edit, "auto".into(), Vec::new());
    app.on_front(FrontMsg::TurnStarted);
    app.queue.extend(["waiting".to_string(), "already taken".to_string(), "waiting".to_string()]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(FrontMsg::Steered("already taken".into())).unwrap();

    let actions = key_after_updates(&mut app, &mut rx, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), Instant::now());

    assert!(app.queue.is_empty());
    assert!(matches!(actions.as_slice(), [Action::Unsteer(first), Action::Unsteer(second), Action::Stop, Action::Print(_)] if first == "waiting" && second == "waiting"));
}
