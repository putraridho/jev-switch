use super::super::keys::continues_line;

#[test]
fn enter_after_one_backslash_continues_the_line() {
    assert!(continues_line("first line\\", 11));
    assert!(!continues_line("first line\\", 3), "only at the end of the line");
    assert!(!continues_line("a literal \\\\", 12), "a doubled backslash is literal");
    assert!(!continues_line("plain", 5));
}
