//! The mouse wheel in full-screen mode.
//!
//! jev-code runs on the terminal's alternate screen and leaves the mouse to the terminal, so text
//! can be selected and links clicked. With the mouse not reported, most terminals send the wheel
//! as Up and Down arrow keys ("alternate scroll mode"): several arrows at once for one notch of
//! the wheel, written together. A person pressing an arrow key sends one, or repeats them about
//! thirty times a second.
//!
//! So arrows that arrive already buffered together, in the same direction, are a wheel: they
//! scroll the chat. A lone arrow is still an arrow (history, moving in the box). Nothing is
//! reported by the terminal, so this is a heuristic, and one wheel notch that sends a single
//! arrow looks like a key press.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};

/// Which way an arrow points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// The direction of a plain Up or Down press (no modifiers: Alt-Up and the like are not the
/// wheel). A key release is not a press.
pub fn arrow(key: &KeyEvent) -> Option<Direction> {
    if key.kind == KeyEventKind::Release || !key.modifiers.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(Direction::Up),
        KeyCode::Down => Some(Direction::Down),
        _ => None,
    }
}

fn arrow_of(event: &Event) -> Option<Direction> {
    match event {
        Event::Key(key) => arrow(key),
        _ => None,
    }
}

/// How many more arrows in `direction` follow at once. `next` yields the next event that is
/// already there (without waiting), or `None`; `put_back` takes an event that was not one of
/// them, so it is handled next, in order. The arrows are consumed.
pub fn following(direction: Direction, next: &mut dyn FnMut() -> Option<Event>, put_back: &mut dyn FnMut(Event)) -> usize {
    let mut count = 0;
    while let Some(event) = next() {
        if arrow_of(&event) == Some(direction) {
            count += 1;
        } else {
            put_back(event);
            break;
        }
    }
    count
}

/// How far to scroll the chat for a burst of `arrows` arrows in `direction`: positive goes back
/// in time (up). One row per arrow, so the speed is the terminal's own wheel setting.
pub fn scroll_rows(direction: Direction, arrows: usize) -> isize {
    let rows = isize::try_from(arrows).unwrap_or(isize::MAX);
    match direction {
        Direction::Up => rows,
        Direction::Down => -rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::collections::VecDeque;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn run(first: Direction, mut queue: VecDeque<Event>) -> (usize, Vec<Event>) {
        let mut put_back = Vec::new();
        let count = following(first, &mut || queue.pop_front(), &mut |e| put_back.push(e));
        (count, put_back)
    }

    #[test]
    fn plain_up_and_down_presses_are_arrows_and_nothing_else_is() {
        assert_eq!(arrow(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)), Some(Direction::Up));
        assert_eq!(arrow(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)), Some(Direction::Down));
        assert_eq!(arrow(&KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)), None, "Alt-Up is a key binding");
        assert_eq!(arrow(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)), None);
        let mut release = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(arrow(&release), None);
    }

    #[test]
    fn a_lone_arrow_is_not_a_wheel() {
        assert_eq!(run(Direction::Up, VecDeque::new()), (0, vec![]));
    }

    #[test]
    fn arrows_in_the_same_direction_that_arrive_together_are_counted() {
        let queue: VecDeque<Event> = [KeyCode::Up, KeyCode::Up].into_iter().map(key).collect();
        assert_eq!(run(Direction::Up, queue), (2, vec![]));
    }

    #[test]
    fn the_burst_stops_at_another_key_and_hands_it_back() {
        let queue: VecDeque<Event> = [KeyCode::Down, KeyCode::Down, KeyCode::Char('x'), KeyCode::Down].into_iter().map(key).collect();
        let (count, put_back) = run(Direction::Down, queue);
        assert_eq!(count, 2);
        assert_eq!(put_back, vec![key(KeyCode::Char('x'))], "the key after the wheel is not lost");
        let opposite: VecDeque<Event> = [KeyCode::Down].into_iter().map(key).collect();
        assert_eq!(run(Direction::Up, opposite), (0, vec![key(KeyCode::Down)]), "the other direction is a new gesture");
    }

    #[test]
    fn up_goes_back_in_time_and_down_comes_forward() {
        assert_eq!(scroll_rows(Direction::Up, 3), 3);
        assert_eq!(scroll_rows(Direction::Down, 3), -3);
    }
}
