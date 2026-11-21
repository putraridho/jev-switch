//! Editor tests: wrapping into rows.

use super::*;

#[test]
fn layout_wraps_by_display_width_and_places_the_cursor() {
    let e = ed("abcdef", 6);
    assert_eq!(e.layout(4), (vec!["abcd".into(), "ef".into()], (1, 2)));
    let full = ed("abcd", 4);
    assert_eq!(full.layout(4), (vec!["abcd".into(), String::new()], (1, 0)));
    let wide = ed("日本語", 6);
    assert_eq!(wide.layout(5), (vec!["日本".into(), "語".into()], (1, 0)));
    let lines = ed("a\nbc", 1);
    assert_eq!(lines.layout(10), (vec!["a".into(), "bc".into()], (0, 1)));
    assert_eq!(Editor::default().layout(10), (vec![String::new()], (0, 0)));
}

#[test]
fn layout_breaks_at_spaces_and_never_inside_a_word_that_fits() {
    let e = ed("the quick brown fox", 19);
    assert_eq!(e.layout(10), (vec!["the quick".into(), "brown fox".into()], (1, 9)));
    // The cursor on the space a row breaks at sits at the end of that row.
    let e = ed("the quick brown fox", 9);
    assert_eq!(e.layout(10), (vec!["the quick".into(), "brown fox".into()], (0, 9)));
    let e = ed("the quick brown fox", 10);
    assert_eq!(e.layout(10).1, (1, 0));
    // A word wider than a row is still cut by column; a row's text is never lost.
    let e = ed("a supercalifragilistic b", 24);
    let (rows, _) = e.layout(8);
    assert_eq!(rows.join(" ").replace("  ", " "), "a supercal ifragili stic b".replace("  ", " "));
    assert!(rows.iter().all(|r| display_width(r) <= 8), "{rows:?}");
}
