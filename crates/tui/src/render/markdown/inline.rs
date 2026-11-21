//! Inline spans (`code`, `**bold**`, links), words and wrapped drawing.

use crate::render::style::Style;
use crate::render::wrap::{push_char, wrap_words, Span, Tok, Word};

/// `s` painted the way `span` is drawn.
pub(super) fn paint_span(style: Style, span: Span, s: &str) -> String {
    match span {
        Span::Plain => s.to_string(),
        Span::Bold => style.bold(s),
        Span::Code => style.cyan(s),
        Span::Dim => style.dim(s),
    }
}

fn push_tok(out: &mut Vec<Tok>, text: &str, span: Span) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(t) if t.span == span => t.text.push_str(text),
        _ => out.push(Tok { text: text.to_string(), span }),
    }
}

fn backtick_run(chars: &[char], at: usize) -> usize {
    chars[at..].iter().take_while(|c| **c == '`').count()
}

/// Start of the next run of exactly `run` backticks at or after `from`.
fn find_backtick_run(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut k = from;
    while k < chars.len() {
        if chars[k] == '`' {
            let m = backtick_run(chars, k);
            if m == run {
                return Some(k);
            }
            k += m;
        } else {
            k += 1;
        }
    }
    None
}

/// Start of a closing `**` for a bold span whose content starts at `from`.
fn find_bold_close(chars: &[char], from: usize) -> Option<usize> {
    if chars.get(from).is_none_or(|c| c.is_whitespace()) {
        return None;
    }
    (from + 1..chars.len().saturating_sub(1))
        .find(|&j| chars[j] == '*' && chars[j + 1] == '*' && !chars[j - 1].is_whitespace())
}

/// For a link whose `[` is at `at`: where its text ends (the `]`) and where its target
/// ends (the `)`). The target has no spaces; text and target are not empty.
fn find_link(chars: &[char], at: usize) -> Option<(usize, usize)> {
    let close = (at + 1..chars.len()).find(|&j| chars[j] == ']' || chars[j] == '[')?;
    if chars[close] != ']' || close == at + 1 || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|&j| chars[j] == ')' || chars[j].is_whitespace())?;
    (chars[end] == ')' && end > close + 2).then_some((close, end))
}

/// `text (url)` with the url dimmed, or just the url when the text is the url.
fn push_link(out: &mut Vec<Tok>, text: &str, url: &str, base: Span) {
    if text == url {
        push_tok(out, url, Span::Dim);
        return;
    }
    for t in parse_inline(text, base) {
        push_tok(out, &t.text, t.span);
    }
    push_tok(out, " ", base);
    push_tok(out, &format!("({url})"), Span::Dim);
}

/// Light inline markdown: `code`, `**bold**`, `[text](url)`. Anything unmatched stays
/// literal.
pub(super) fn parse_inline(s: &str, base: Span) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let (mut out, mut buf, mut i) = (Vec::new(), String::new(), 0);
    while i < chars.len() {
        if chars[i] == '`' {
            let run = backtick_run(&chars, i);
            if let Some(j) = find_backtick_run(&chars, i + run, run) {
                let code: String = chars[i + run..j].iter().collect();
                if !code.trim().is_empty() {
                    push_tok(&mut out, &std::mem::take(&mut buf), base);
                    push_tok(&mut out, code.trim(), Span::Code);
                    i = j + run;
                    continue;
                }
            }
            buf.extend(std::iter::repeat_n('`', run));
            i += run;
        } else if let Some((close, end)) = if chars[i] == '[' { find_link(&chars, i) } else { None } {
            push_tok(&mut out, &std::mem::take(&mut buf), base);
            let text: String = chars[i + 1..close].iter().collect();
            let url: String = chars[close + 2..end].iter().collect();
            push_link(&mut out, &text, &url, base);
            i = end + 1;
        } else if base != Span::Bold && chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(j) = find_bold_close(&chars, i + 2) {
                push_tok(&mut out, &std::mem::take(&mut buf), base);
                let inner: String = chars[i + 2..j].iter().collect();
                for t in parse_inline(&inner, Span::Bold) {
                    push_tok(&mut out, &t.text, t.span);
                }
                i = j + 2;
            } else {
                buf.push_str("**");
                i += 2;
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    push_tok(&mut out, &buf, base);
    out
}

pub(super) fn to_words(toks: &[Tok]) -> Vec<Word> {
    let (mut words, mut cur): (Vec<Word>, Word) = (Vec::new(), Vec::new());
    for t in toks {
        for c in t.text.chars() {
            if c.is_whitespace() {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                push_char(&mut cur, c, t.span);
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

pub(super) fn render_words(row: &[Word], style: Style) -> String {
    row.iter()
        .map(|w| w.iter().map(|t| paint_span(style, t.span, &t.text)).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wrap `words` to `avail` columns and draw them, with `first` before the first row and
/// `rest` before the others.
pub(super) fn draw_wrapped(words: &[Word], avail: usize, first: &str, rest: &str, style: Style) -> Vec<String> {
    wrap_words(words, avail)
        .iter()
        .enumerate()
        .map(|(i, row)| format!("{}{}", if i == 0 { first } else { rest }, render_words(row, style)))
        .collect()
}
