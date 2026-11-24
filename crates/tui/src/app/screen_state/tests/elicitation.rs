use super::*;
use jev_backend::{ElicitationRequest, ElicitationKind, ElicitationField, ElicitationAnswer, ElicitationValue, FieldKind};

fn question(a: &mut App, fields: bool) -> (oneshot::Receiver<ElicitationAnswer>, Instant) {
    let (reply, rx) = oneshot::channel();
    let field = ElicitationField { name: "approve".into(), title: "Approve".into(), description: "Run it".into(), required: true, kind: FieldKind::Boolean, default: "false".into(), min: None, max: None, min_length: None, max_length: None };
    a.on_front(FrontMsg::Elicitation { request: ElicitationRequest { id: "q".into(), server: "lunagraph".into(), message: "Run command?".into(), kind: ElicitationKind::Form(if fields { vec![field] } else { vec![] }) }, reply });
    (rx, Instant::now() + CARD_GRACE)
}

#[test]
fn a_form_returns_typed_values_without_touching_the_message_box_or_queue() {
    let mut a = app();
    a.editor.set("my queued draft");
    a.mode = Mode::Auto;
    let (mut rx, now) = question(&mut a, true);
    a.on_key(key(KeyCode::Down), now); // false -> true
    a.on_key(key(KeyCode::Enter), now); // review
    assert!(rx.try_recv().is_err());
    a.on_key(key(KeyCode::Up), now); // decline -> send
    assert!(a.on_key(key(KeyCode::Enter), now).is_empty());
    assert_eq!(rx.try_recv().unwrap(), ElicitationAnswer::Accept(vec![("approve".into(), ElicitationValue::Boolean(true))]));
    assert_eq!(a.editor.text(), "my queued draft");
    assert!(a.queue.is_empty());
}

#[test]
fn missing_boolean_stays_on_the_field_and_shows_an_error() {
    let mut a = app();
    let (mut rx, now) = question(&mut a, true);
    a.elicitation.as_mut().unwrap().editor.set("");
    a.on_key(key(KeyCode::Enter), now);
    assert!(a.elicitation.as_ref().unwrap().error.is_some());
    assert!(rx.try_recv().is_err());
    a.on_paste("true");
    assert_eq!(a.elicitation.as_ref().unwrap().editor.text(), "true");
}

#[test]
fn decline_cancel_interrupt_and_withdrawal_are_distinct() {
    for (key, expected, stop) in [(key(KeyCode::Enter), ElicitationAnswer::Decline, false), (key(KeyCode::Esc), ElicitationAnswer::Cancel, false), (with(KeyCode::Char('c'), KeyModifiers::CONTROL), ElicitationAnswer::Cancel, true)] {
        let mut a = app();
        let (mut rx, now) = question(&mut a, false);
        let actions = a.on_key(key, now);
        assert_eq!(rx.try_recv().unwrap(), expected);
        assert_eq!(actions.iter().any(|a| matches!(a, Action::Stop)), stop);
        assert!(a.elicitation.is_none());
    }
    let mut a = app();
    let (mut rx, _) = question(&mut a, true);
    a.on_front(FrontMsg::ElicitationGone);
    assert!(a.elicitation.is_none());
    assert!(rx.try_recv().is_err());
}

#[test]
fn typing_ahead_cannot_answer_and_back_allows_editing() {
    let mut a = app();
    let (mut rx, now) = question(&mut a, true);
    a.on_key(key(KeyCode::Enter), now - CARD_GRACE);
    assert!(a.elicitation.as_ref().unwrap().editing());
    a.on_key(key(KeyCode::Enter), now);
    a.on_key(key(KeyCode::BackTab), now);
    assert!(a.elicitation.as_ref().unwrap().editing());
    assert_eq!(a.elicitation.as_ref().unwrap().editor.text(), "false");
    assert!(rx.try_recv().is_err());
}
