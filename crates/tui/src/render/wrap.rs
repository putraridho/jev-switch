//! Word wrapping to a column width. The word and span types here are also what the
//! markdown renderer wraps, so a styled run survives being cut across rows.

use crate::render::width::display_width;

fn is_url(word: &str) -> bool {
    word.contains("://") || word.starts_with("www.")
}

/// How a run of words is drawn. Only answer text uses more than `Plain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Span {
    Plain,
    Bold,
    Code,
    /// The target of a link: dimmed, and never broken across rows.
    Dim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tok {
    pub(super) text: String,
    pub(super) span: Span,
}

/// An unbreakable run: pieces of different spans glued with no space between them
/// (`**bold**,` is bold text then a plain comma, one word).
pub(super) type Word = Vec<Tok>;

pub(super) fn push_char(word: &mut Word, c: char, span: Span) {
    match word.last_mut() {
        Some(t) if t.span == span => t.text.push(c),
        _ => word.push(Tok { text: c.to_string(), span }),
    }
}

pub(super) fn word_width(w: &Word) -> usize {
    w.iter().map(|t| display_width(&t.text)).sum()
}

pub(super) fn word_text(w: &Word) -> String {
    w.iter().map(|t| t.text.as_str()).collect()
}

/// Cut a word into pieces of at most `avail` columns.
fn split_word(word: &Word, avail: usize) -> Vec<Word> {
    let (mut out, mut cur, mut plain): (Vec<Word>, Word, String) = (Vec::new(), Vec::new(), String::new());
    for t in word {
        for c in t.text.chars() {
            let before = display_width(&plain);
            plain.push(c);
            if before > 0 && display_width(&plain) > avail {
                out.push(std::mem::take(&mut cur));
                plain = c.to_string();
            }
            push_char(&mut cur, c, t.span);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Greedy word wrap into lines of at most `avail` columns. A URL is never cut (it may
/// overflow); any other word longer than a line is split by column.
pub(super) fn wrap_words(words: &[Word], avail: usize) -> Vec<Vec<Word>> {
    let avail = avail.max(1);
    let (mut lines, mut cur, mut cw): (Vec<Vec<Word>>, Vec<Word>, usize) = (Vec::new(), Vec::new(), 0);
    for word in words {
        let pieces = if word_width(word) > avail && !is_url(&word_text(word)) && !word.iter().all(|t| t.span == Span::Dim) {
            split_word(word, avail)
        } else {
            vec![word.clone()]
        };
        for piece in pieces {
            let pw = word_width(&piece);
            if cur.is_empty() {
                cw = pw;
                cur.push(piece);
            } else if cw + 1 + pw <= avail {
                cw += 1 + pw;
                cur.push(piece);
            } else {
                lines.push(std::mem::take(&mut cur));
                cw = pw;
                cur.push(piece);
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn leading_indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Plain `text` word-wrapped to `width` columns, one `String` per output line. Existing
/// newlines are kept; a line that already fits is returned as it is; a wrapped line
/// keeps its leading indentation on every row. URLs are never broken; other words wider
/// than the line are split by column. Plain text only: escape sequences are not
/// carried across rows.
pub fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if display_width(line) <= width {
            out.push(line.trim_end().to_string());
            continue;
        }
        let mut indent = leading_indent(line).replace('\t', "    ");
        if display_width(&indent) * 2 >= width {
            indent.clear();
        }
        let words: Vec<Word> =
            line.split_whitespace().map(|w| vec![Tok { text: w.to_string(), span: Span::Plain }]).collect();
        let avail = width - display_width(&indent);
        for row in wrap_words(&words, avail) {
            let body: Vec<String> = row.iter().map(word_text).collect();
            out.push(format!("{indent}{}", body.join(" ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_basic_and_indent() {
        assert_eq!(wrap_to_width("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap_to_width("short", 80), vec!["short"]);
        assert_eq!(wrap_to_width("", 10), vec![""]);
        assert_eq!(wrap_to_width("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap_to_width("  aaa bbb ccc", 9), vec!["  aaa bbb", "  ccc"]);
        // a deep indent that leaves no room is dropped rather than wrapping forever
        assert!(wrap_to_width(&format!("{}x y z", " ".repeat(30)), 10).iter().all(|l| display_width(l) <= 10));
    }

    #[test]
    fn wrap_never_breaks_urls_but_splits_other_long_words() {
        let url = "https://example.com/a/very/long/path/that/does/not/fit";
        let out = wrap_to_width(&format!("see {url} now"), 20);
        assert_eq!(out, vec!["see", url, "now"]);
        let out = wrap_to_width("abcdefghijklmnop", 5);
        assert_eq!(out, vec!["abcde", "fghij", "klmno", "p"]);
    }

    #[test]
    fn wrap_counts_columns_not_bytes() {
        let out = wrap_to_width("日本語 日本語 日本語", 7);
        assert_eq!(out, vec!["日本語", "日本語", "日本語"]);
        for w in 1..20 {
            for l in wrap_to_width("🚀 launch 日本語のテキスト and e\u{301}cole", w) {
                assert!(display_width(&l) <= w.max(2), "{l:?} at {w}");
            }
        }
    }
}
