use crate::tui::render::diffview::word_diff::{word_diff, Piece};

#[test]
fn word_diff_marks_only_the_changed_words() {
    let (l, r) = word_diff("let total = price * 3;", "let total = price * quantity;").unwrap();
    let changed = |p: &[Piece]| {
        p.iter()
            .filter(|p| p.changed)
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(changed(&l), ["3"]);
    assert_eq!(changed(&r), ["quantity"]);
    assert_eq!(
        l.iter().map(|p| p.text.as_str()).collect::<String>(),
        "let total = price * 3;"
    );
}

#[test]
fn unrelated_lines_get_no_word_marks() {
    assert!(word_diff("alpha beta gamma", "zzz").is_none());
    assert!(
        word_diff(&"x ".repeat(300), "y").is_none(),
        "too long to compare"
    );
}
