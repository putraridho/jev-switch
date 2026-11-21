use super::*;

#[test]
fn unified_has_line_numbers_markers_and_text_without_colour() {
    let out = render_diff(&parse(DIFF), &opts(80, Layout::Unified), OFF);
    no_esc(&out);
    let l = lines(&out);
    assert_eq!(l[0], "▸ src/a.rs  +2 −1");
    assert_eq!(l[1], "  @@ -9,4 +9,5 @@ fn main");
    assert_eq!(l[2], "   9  9   let a = 1;");
    assert_eq!(l[3], "  10    - let total = price * 3;");
    assert_eq!(l[4], "     10 + let total = price * quantity;");
    assert_eq!(l[5], "     11 + let extra = 0;");
    assert_eq!(l[6], "  11 12   done();");
}

#[test]
fn colour_adds_escapes_but_never_changes_the_text() {
    let plain = render_diff(&parse(DIFF), &opts(80, Layout::Unified), OFF);
    let coloured = render_diff(&parse(DIFF), &opts(80, Layout::Unified), ON);
    assert!(coloured.contains('\x1b'));
    assert_eq!(strip_ansi(&coloured), plain);
}

#[test]
fn changed_words_are_reversed_and_context_is_syntax_coloured() {
    let out = render_diff(&parse(DIFF), &opts(80, Layout::Unified), ON);
    assert!(
        out.contains("\x1b[7m"),
        "reverse video marks the words that changed: {out:?}"
    );
    let ctx = out
        .lines()
        .find(|l| strip_ansi(l).contains("let a = 1;"))
        .unwrap();
    assert!(
        ctx.contains("\x1b[") && ctx.matches('\x1b').count() > 6,
        "keywords and numbers coloured in {ctx:?}"
    );
}
