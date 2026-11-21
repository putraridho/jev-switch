//! Type-ahead tests: what a key does to the draft.

use super::*;

#[test]
fn enter_queues_and_esc_stops_with_what_was_typed() {
    let mut d = Draft::default();
    assert_eq!(d.apply(Key::Enter), Effect::Nothing, "an empty Enter queues nothing");
    assert_eq!(d.apply(text("run the tests ")), Effect::Changed);
    assert_eq!(d.apply(Key::Enter), Effect::Queued("run the tests".into()));
    assert_eq!(d.text, "");
    assert_eq!(d.apply(Key::Esc), Effect::Stop(None));
    d.apply(text("do this instead"));
    assert_eq!(d.apply(Key::Esc), Effect::Stop(Some("do this instead".into())));
}

#[test]
fn editing_keys_change_the_draft() {
    let mut d = Draft { text: "fix the  bug".into() };
    assert_eq!(d.apply(Key::DeleteWord), Effect::Changed);
    assert_eq!(d.text, "fix the  ");
    d.apply(Key::DeleteWord);
    assert_eq!(d.text, "fix ");
    d.apply(Key::Backspace);
    assert_eq!(d.text, "fix");
    assert_eq!(d.apply(Key::ClearLine), Effect::Changed);
    assert_eq!(d.apply(Key::ClearLine), Effect::Nothing);
    assert_eq!(d.apply(Key::Backspace), Effect::Nothing);
}
