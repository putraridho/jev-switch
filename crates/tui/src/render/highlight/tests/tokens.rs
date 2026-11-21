//! Highlighter tests: colouring of each kind of token.

use super::*;

#[test]
fn rust_colours_keywords_and_strings() {
    let out = hl(Lang::Rust, "fn main() { let s = \"hi\"; }", ON);
    assert!(out.contains("\x1b[35mfn\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[35mlet\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[32m\"hi\"\x1b[0m"), "{out:?}");
    assert!(!out.contains("\x1b[35mmain"), "identifiers are not keywords");
}

#[test]
fn numbers_and_comments_are_coloured() {
    let out = hl(Lang::Rust, "let x = 42; // answer", ON);
    assert!(out.contains("\x1b[33m42\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[2m// answer\x1b[0m"), "{out:?}");
    // a digit inside a name is part of the name
    assert!(!hl(Lang::Rust, "x1", ON).contains('\x1b'));
}

#[test]
fn a_comment_marker_inside_a_string_is_text() {
    let out = hl(Lang::Rust, "let u = \"http://x\";", ON);
    assert!(out.contains("\x1b[32m\"http://x\"\x1b[0m"), "{out:?}");
    assert!(!out.contains("\x1b[2m"));
}

#[test]
fn escaped_quotes_stay_inside_the_string() {
    let out = hl(Lang::Python, r#"x = "a\"b" + 1"#, ON);
    assert!(out.contains(r#"[32m"a\"b"[0m"#.replace('[', "\x1b[").as_str()), "{out:?}");
}

#[test]
fn hash_comments_need_a_word_start() {
    assert!(hl(Lang::Shell, "ls # list", ON).contains("\x1b[2m# list\x1b[0m"));
    assert!(!hl(Lang::Shell, "echo $#", ON).contains("\x1b[2m"));
    assert!(hl(Lang::Python, "# top", ON).contains("\x1b[2m# top\x1b[0m"));
}

#[test]
fn block_comments_carry_across_lines() {
    let mut state = LineState::Normal;
    let first = highlight_line(Lang::C, "int a; /* start", &mut state, ON);
    assert_eq!(state, LineState::InBlockComment);
    assert!(first.contains("\x1b[2m/* start\x1b[0m"), "{first:?}");
    let mid = highlight_line(Lang::C, "still comment", &mut state, ON);
    assert_eq!(mid, "\x1b[2mstill comment\x1b[0m");
    let last = highlight_line(Lang::C, "end */ return 1;", &mut state, ON);
    assert_eq!(state, LineState::Normal);
    assert!(last.starts_with("\x1b[2mend */\x1b[0m"), "{last:?}");
    assert!(last.contains("\x1b[35mreturn\x1b[0m"), "{last:?}");
}

#[test]
fn sql_keywords_ignore_case_and_use_dash_comments() {
    let out = hl(Lang::Sql, "Select name FROM t -- all", ON);
    assert!(out.contains("\x1b[35mSelect\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[35mFROM\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[2m-- all\x1b[0m"), "{out:?}");
}

#[test]
fn json_colours_strings_numbers_and_literals() {
    let out = hl(Lang::Json, "{\"a\": [1, null]}", ON);
    assert!(out.contains("\x1b[32m\"a\"\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[33m1\x1b[0m"), "{out:?}");
    assert!(out.contains("\x1b[35mnull\x1b[0m"), "{out:?}");
}

#[test]
fn diff_colours_added_removed_and_hunk_lines() {
    assert_eq!(hl(Lang::Diff, "+added", ON), "\x1b[32m+added\x1b[0m");
    assert_eq!(hl(Lang::Diff, "-removed", ON), "\x1b[31m-removed\x1b[0m");
    assert_eq!(hl(Lang::Diff, "@@ -1,2 +1,2 @@", ON), "\x1b[36m@@ -1,2 +1,2 @@\x1b[0m");
    assert_eq!(hl(Lang::Diff, " context", ON), "\x1b[2m context\x1b[0m");
}
