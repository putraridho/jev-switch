//! Type-ahead tests: what a chunk of input means.

use super::*;

#[test]
fn a_chunk_becomes_keys() {
    assert_eq!(keys(b"fix it"), [text("fix it")]);
    assert_eq!(keys(b"\r"), [Key::Enter]);
    assert_eq!(keys(b"ab\x7f"), [text("ab"), Key::Backspace]);
    assert_eq!(keys(b"\x15\x17"), [Key::ClearLine, Key::DeleteWord]);
    assert_eq!(keys("é日".as_bytes()), [text("é日")]);
}

#[test]
fn escapes_are_esc_newline_or_nothing() {
    assert_eq!(keys(b"\x1b"), [Key::Esc]);
    assert_eq!(keys(b"\x1b\r"), [Key::Newline], "Alt-Enter (and Shift-Enter after /terminal-setup)");
    assert_eq!(keys(b"a\x1b[Ab"), [text("a"), text("b")], "an arrow key is dropped");
    assert_eq!(keys(b"\x1b[1;5C"), Vec::<Key>::new());
    assert_eq!(keys(b"\x1bOP"), Vec::<Key>::new(), "F1");
    assert_eq!(keys(b"\x1bx"), Vec::<Key>::new(), "Alt-x");
    assert_eq!(keys(b"\x1b\x1b"), [Key::Esc, Key::Esc]);
}

#[test]
fn a_pasted_block_is_one_message() {
    assert_eq!(keys(b"one\ntwo\n"), [text("one"), Key::Newline, text("two"), Key::Enter]);
    assert_eq!(keys(b"one\r\ntwo"), [text("one"), Key::Newline, text("two")]);
}
