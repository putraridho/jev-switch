//! Editor tests: typing, pasting, deleting and completion.

use super::*;

#[test]
fn typing_and_deleting_keep_the_cursor_on_character_boundaries() {
    let mut e = Editor::default();
    for c in "héllo 日本".chars() {
        e.insert_char(c);
    }
    assert_eq!(e.text(), "héllo 日本");
    e.backspace();
    e.left();
    e.left();
    e.delete();
    assert_eq!(e.text(), "héllo日");
    e.home();
    e.right();
    e.backspace();
    assert_eq!((e.text(), e.cursor()), ("éllo日", 0));
}

#[test]
fn dd_leaves_a_valid_cursor_on_every_kind_of_line() {
    // First line: the next line takes its place.
    let mut e = ed("ab\ncd", 1);
    e.delete_line();
    assert_eq!((e.before_cursor(), e.text(), e.cursor()), ("", "cd", 0));

    // Middle line.
    let mut e = ed("ab\ncd\nef", 4);
    e.delete_line();
    assert_eq!((e.before_cursor(), e.text(), e.cursor()), ("ab\n", "ab\nef", 3));

    // Last line: the cursor used to stay past the new end of the text, so `before_cursor`
    // panicked.
    let mut e = ed("ab\ncd", 5);
    e.delete_line();
    assert_eq!((e.before_cursor(), e.text(), e.cursor()), ("", "ab", 0));

    // The only line.
    let mut e = ed("only", 4);
    e.delete_line();
    assert_eq!((e.before_cursor(), e.text(), e.cursor()), ("", "", 0));
}

#[test]
fn paste_keeps_line_breaks_and_drops_control_characters() {
    let mut e = Editor::default();
    e.insert_str("a\r\nb\tc\x07\rd");
    assert_eq!(e.text(), "a\nb    c\nd");
}

#[test]
fn completion_replaces_the_word_before_the_cursor() {
    let mut e = ed("/mo", 3);
    e.replace_before_cursor(0, "/model ");
    assert_eq!((e.text(), e.cursor()), ("/model ", 7));
}
