use super::*;

fn ask(always: bool) -> (ApprovalAsk, oneshot::Receiver<AnswerKey>) {
    let (reply, rx) = oneshot::channel();
    let ask = ApprovalAsk {
        tool: "Bash".into(),
        notes: Vec::new(),
        lines: vec!["cargo test".into()],
        always: always.then(|| "Bash(cargo test:*)".to_string()),
        note: None,
        reply,
    };
    (ask, rx)
}

#[test]
fn the_approval_card_answers_by_number_letter_arrows_or_esc() {
    for (keys, expected) in [
        (vec![key(KeyCode::Char('1'))], AnswerKey::Yes),
        (vec![key(KeyCode::Char('2'))], AnswerKey::Always),
        (vec![key(KeyCode::Char('3'))], AnswerKey::No),
        (vec![key(KeyCode::Char('a'))], AnswerKey::Always),
        (vec![key(KeyCode::Esc)], AnswerKey::No),
        (vec![key(KeyCode::Down), key(KeyCode::Enter)], AnswerKey::Always),
        (vec![key(KeyCode::Up), key(KeyCode::Enter)], AnswerKey::No),
        (vec![with(KeyCode::Char('c'), KeyModifiers::CONTROL)], AnswerKey::Interrupt),
    ] {
        let mut a = app();
        let (q, mut rx) = ask(true);
        a.on_front(FrontMsg::Approval(Box::new(q)));
        let later = Instant::now() + CARD_GRACE;
        for k in keys {
            a.on_key(k, later);
        }
        assert_eq!(rx.try_recv().ok(), Some(expected));
        assert!(a.approval.is_none());
    }
}

#[test]
fn without_always_on_offer_a_and_2_do_not_save_a_rule() {
    let mut a = app();
    let (q, mut rx) = ask(false);
    a.on_front(FrontMsg::Approval(Box::new(q)));
    let later = Instant::now() + CARD_GRACE;
    a.on_key(key(KeyCode::Char('a')), later);
    assert!(rx.try_recv().is_err(), "a is not an answer");
    a.on_key(key(KeyCode::Char('2')), later);
    assert_eq!(rx.try_recv().ok(), Some(AnswerKey::No), "2 is the second choice: No");
}

#[test]
fn keys_typed_ahead_never_answer_a_card_that_just_appeared_but_ctrl_c_stops() {
    let mut a = app();
    let (q, mut rx) = ask(true);
    a.on_front(FrontMsg::Approval(Box::new(q)));
    a.on_key(key(KeyCode::Enter), Instant::now());
    a.on_key(key(KeyCode::Char('y')), Instant::now());
    assert!(rx.try_recv().is_err() && a.approval.is_some(), "Enter or y right away is typing ahead");
    a.on_key(with(KeyCode::Char('c'), KeyModifiers::CONTROL), Instant::now());
    assert_eq!(rx.try_recv().ok(), Some(AnswerKey::Interrupt));
}

#[test]
fn a_withdrawn_question_is_dropped_and_the_key_goes_to_the_box() {
    let mut a = app();
    let (q, rx) = ask(true);
    a.on_front(FrontMsg::Approval(Box::new(q)));
    drop(rx);
    typed(&mut a, "y");
    assert!(a.approval.is_none());
    assert_eq!(a.editor.text(), "y");
}

#[test]
fn a_confirm_defaults_to_no_and_takes_y() {
    let mut a = app();
    let (reply, rx) = std_mpsc::channel();
    a.on_front(FrontMsg::Confirm { question: "Revert 2 files?".into(), reply });
    a.on_key(key(KeyCode::Enter), Instant::now());
    assert_eq!(rx.try_recv().ok(), Some(false));
    let (reply, rx) = std_mpsc::channel();
    a.on_front(FrontMsg::Confirm { question: "Revert?".into(), reply });
    a.on_key(key(KeyCode::Char('y')), Instant::now());
    assert_eq!(rx.try_recv().ok(), Some(true));
}
