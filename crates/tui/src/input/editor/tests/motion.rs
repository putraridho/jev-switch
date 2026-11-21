//! Editor tests: moving the cursor and killing words.

use super::*;

#[test]
fn word_moves_and_kills() {
    let mut e = ed("cargo test --quiet", 18);
    e.word_left();
    assert_eq!(e.cursor(), 13);
    e.delete_word_back();
    assert_eq!(e.text(), "cargo quiet");
    e.word_right();
    assert_eq!(e.cursor(), 11);
    e.home();
    e.word_right();
    assert_eq!(e.cursor(), 5);
    e.kill_to_end();
    assert_eq!(e.text(), "cargo");
    e.kill_to_start();
    assert_eq!((e.text(), e.cursor()), ("", 0));
}

#[test]
fn up_and_down_move_between_lines_and_say_when_they_cannot() {
    let mut e = ed("first line\nsecond", 17);
    assert!(!e.down());
    assert!(e.up());
    assert_eq!(e.cursor(), 6);
    assert!(!e.up());
    assert!(e.down());
    assert_eq!(e.cursor(), 17);
}
