//! Assistant text with light markdown for a styled terminal.

use super::ansi::strip_ansi;
use super::style::Style;
use super::width::{clamp_width, display_width};
use super::wrap::{push_char, wrap_words, Span, Tok, Word};

/// `s` painted the way `span` is drawn.
fn paint_span(style: Style, span: Span, s: &str) -> String {
    match span {
        Span::Plain => s.to_string(),
        Span::Bold => style.bold(s),
        Span::Code => style.cyan(s),
    }
}

/// Remove escape sequences and control characters (except tab) from model text, so
/// nothing the model prints can drive the terminal.
fn sanitize_answer_line(line: &str) -> String {
    strip_ansi(line.trim_end_matches('\r')).chars().filter(|c| !c.is_control() || *c == '\t').collect()
}

fn expand_leading_tabs(line: &str) -> String {
    let rest = line.trim_start_matches('\t');
    format!("{}{rest}", "    ".repeat(line.len() - rest.len()))
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

/// Light inline markdown: `code`, `**bold**`. Anything unmatched stays literal.
fn parse_inline(s: &str, base: Span) -> Vec<Tok> {
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

fn to_words(toks: &[Tok]) -> Vec<Word> {
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

fn render_words(row: &[Word], style: Style) -> String {
    row.iter()
        .map(|w| w.iter().map(|t| paint_span(style, t.span, &t.text)).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(indent, marker, rest)` of a list item or quote line.
fn list_marker(line: &str) -> Option<(usize, &str, &str)> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    for m in ["- ", "* ", "+ ", "> "] {
        if let Some(rest) = trimmed.strip_prefix(m) {
            return Some((indent, &trimmed[..1], rest));
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    let after = &trimmed[digits..];
    if (1..=3).contains(&digits) && (after.starts_with(". ") || after.starts_with(") ")) {
        return Some((indent, &trimmed[..digits + 1], &after[2..]));
    }
    None
}

/// `(fence char, length, info string)` when `line` opens or closes a code fence.
fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start_matches(' ');
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    let info = t[n..].trim();
    (n >= 3 && !(c == '`' && info.contains('`'))).then_some((c, n, info))
}

/// Wrap `words` to `avail` columns and draw them, with `first` before the first row and
/// `rest` before the others.
fn draw_wrapped(words: &[Word], avail: usize, first: &str, rest: &str, style: Style) -> Vec<String> {
    wrap_words(words, avail)
        .iter()
        .enumerate()
        .map(|(i, row)| format!("{}{}", if i == 0 { first } else { rest }, render_words(row, style)))
        .collect()
}

/// Assistant text with LIGHT markdown for a styled terminal: `#` headings bold,
/// `**bold**`, inline `code` cyan, fenced code blocks dimmed and indented under their
/// language, list items kept with a hanging indent, quotes, and prose word-wrapped to
/// `width`. Code blocks, table rows and URLs are never wrapped or broken; any other
/// word wider than the line is split by column. An unclosed fence treats the rest as
/// code. Escape sequences and control characters in the text are removed. With a
/// disabled style the text is returned UNCHANGED, so pipes and logs stay raw.
pub fn answer_text(text: &str, style: Style, width: usize) -> String {
    if !style.enabled {
        // Colour off: keep the text as written, but never pass control sequences to a
        // terminal (NO_COLOR, --no-color and TERM=dumb are still terminals).
        return if style.terminal { text.split('\n').map(sanitize_answer_line).collect::<Vec<_>>().join("\n") } else { text.to_string() };
    }
    let width = clamp_width(width);
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for raw in text.split('\n') {
        let line = expand_leading_tabs(&sanitize_answer_line(raw));
        if let Some((fc, fl)) = fence {
            match fence_of(&line) {
                Some((c, n, info)) if c == fc && n >= fl && info.is_empty() => fence = None,
                _ if line.trim().is_empty() => out.push(String::new()),
                _ => out.push(format!("    {}", style.dim(line.trim_end()))),
            }
            continue;
        }
        if let Some((c, n, info)) = fence_of(&line) {
            fence = Some((c, n));
            if let Some(lang) = info.split_whitespace().next() {
                out.push(format!("  {}", style.dim(lang)));
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            out.push(String::new());
        } else if trimmed.starts_with('|') {
            out.push(line.trim_end().to_string());
        } else if let Some(h) = heading_text(trimmed) {
            let words = to_words(&parse_inline(h, Span::Bold));
            out.extend(draw_wrapped(&words, width, "", "", style));
        } else if let Some((indent, marker, rest)) = list_marker(&line) {
            let indent = if indent * 2 >= width { 0 } else { indent };
            let first = format!("{}{marker} ", " ".repeat(indent));
            let hang = " ".repeat(display_width(&first));
            let words = to_words(&parse_inline(rest, Span::Plain));
            out.extend(draw_wrapped(&words, width.saturating_sub(display_width(&first)), &first, &hang, style));
        } else {
            let lead = line.len() - trimmed.len();
            let indent = if lead * 2 >= width { 0 } else { lead };
            let pad = " ".repeat(indent);
            let words = to_words(&parse_inline(trimmed, Span::Plain));
            out.extend(draw_wrapped(&words, width - indent, &pad, &pad, style));
        }
    }
    out.join("\n")
}

/// The text of a `# heading` line (1 to 6 hashes then a space), if it is one.
fn heading_text(trimmed: &str) -> Option<&str> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    let text = rest.trim().trim_end_matches('#').trim_end();
    ((1..=6).contains(&hashes) && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::MIN_WIDTH;
    use crate::uifmt::testutil::*;

    #[test]
    fn answer_text_is_untouched_without_style() {
        for t in [
            "# Title\n**bold** and `code`\n```rust\nfn main() {}\n```\n",
            "",
            "unclosed ```\n\x1b[31mraw\x1b[0m escapes stay in logs",
            "日本語 🚀 e\u{301}",
            &"word ".repeat(200),
        ] {
            assert_eq!(answer_text(t, OFF, 20), t);
        }
    }

    #[test]
    fn answer_headings_bold_and_code() {
        assert_eq!(answer_text("# Title", ON, 80), "\x1b[1mTitle\x1b[0m");
        assert_eq!(answer_text("### Deep ###", ON, 80), "\x1b[1mDeep\x1b[0m");
        assert_eq!(answer_text("#nospace", ON, 80), "#nospace");
        assert_eq!(answer_text("#", ON, 80), "#");
        assert_eq!(answer_text("####### seven", ON, 80), "####### seven");
        assert_eq!(answer_text("use **this** now", ON, 80), "use \x1b[1mthis\x1b[0m now");
        assert_eq!(answer_text("run `cargo test` now", ON, 80), "run \x1b[36mcargo\x1b[0m \x1b[36mtest\x1b[0m now");
        assert_eq!(answer_text("**bold**, then `x`.", ON, 80), "\x1b[1mbold\x1b[0m, then \x1b[36mx\x1b[0m.");
        assert_eq!(answer_text("**bold with `code`**", ON, 80), "\x1b[1mbold\x1b[0m \x1b[1mwith\x1b[0m \x1b[36mcode\x1b[0m");
        // unmatched markers are literal
        assert_eq!(answer_text("a ** b", ON, 80), "a ** b");
        assert_eq!(answer_text("a ` b", ON, 80), "a ` b");
        assert_eq!(answer_text("**unclosed bold", ON, 80), "**unclosed bold");
        assert_eq!(answer_text("2 ** 3 ** 4", ON, 80), "2 ** 3 ** 4");
        assert_eq!(answer_text("``a`b``", ON, 80), "\x1b[36ma`b\x1b[0m");
        assert_eq!(answer_text("****", ON, 80), "****");
    }

    #[test]
    fn answer_fenced_code() {
        let t = "before\n```rust\nfn main() {\n\n    let x = 1;\n}\n```\nafter";
        assert_eq!(
            answer_text(t, ON, 80),
            "before\n  \x1b[2mrust\x1b[0m\n    \x1b[2mfn main() {\x1b[0m\n\n    \x1b[2m    let x = 1;\x1b[0m\n    \x1b[2m}\x1b[0m\nafter"
        );
        // no language, tilde fence, longer closing fence
        assert_eq!(answer_text("~~~\ncode\n~~~~", ON, 80), "    \x1b[2mcode\x1b[0m");
        // markdown inside code is not interpreted and not wrapped
        let long = format!("```\n# not a heading **x** {}\n```", "word ".repeat(40));
        let out = answer_text(&long, ON, 30);
        assert!(strip_ansi(&out).contains("# not a heading **x**"));
        assert_eq!(out.matches('\n').count(), 0, "one code line, not wrapped");
        // a fence shorter than the opening one does not close it
        assert_eq!(answer_text("````\n```\nstill code\n````\nprose", ON, 80), "    \x1b[2m```\x1b[0m\n    \x1b[2mstill code\x1b[0m\nprose");
        // a closing fence cannot carry an info string
        assert_eq!(answer_text("```\n```rust\n```", ON, 80), "    \x1b[2m```rust\x1b[0m");
        // triple backticks inline are not a fence
        assert_eq!(answer_text("use ```x``` here", ON, 80), "use \x1b[36mx\x1b[0m here");
    }

    #[test]
    fn answer_unclosed_fence_swallows_the_rest_as_code() {
        let out = answer_text("```py\nprint(1)\n# still code\n**not bold**", ON, 80);
        assert_eq!(
            out,
            "  \x1b[2mpy\x1b[0m\n    \x1b[2mprint(1)\x1b[0m\n    \x1b[2m# still code\x1b[0m\n    \x1b[2m**not bold**\x1b[0m"
        );
        assert_eq!(answer_text("```", ON, 80), "");
    }

    #[test]
    fn answer_lists_keep_markers_and_hang() {
        assert_eq!(answer_text("- one\n* two\n+ three", ON, 80), "- one\n* two\n+ three");
        assert_eq!(answer_text("1. first\n12) second", ON, 80), "1. first\n12) second");
        assert_eq!(answer_text("  - nested **b**", ON, 80), "  - nested \x1b[1mb\x1b[0m");
        let out = answer_text("- alpha beta gamma delta epsilon zeta", ON, 20);
        assert_eq!(out, "- alpha beta gamma\n  delta epsilon zeta");
        let out = answer_text("10. alpha beta gamma delta", ON, 20);
        assert_eq!(out, "10. alpha beta gamma\n    delta");
        // a bold line starting with ** is not a bullet
        assert_eq!(answer_text("**Note** here", ON, 80), "\x1b[1mNote\x1b[0m here");
        // "-5 degrees" is prose
        assert_eq!(answer_text("-5 degrees", ON, 80), "-5 degrees");
        // quotes keep their marker
        assert_eq!(answer_text("> quoted text", ON, 80), "> quoted text");
    }

    #[test]
    fn answer_wraps_prose_not_urls_tables_or_blank_lines() {
        let url = "https://example.com/some/really/long/path/that/never/fits/on/one/line";
        let out = answer_text(&format!("see {url} for more details on this"), ON, 30);
        assert!(out.contains(url));
        assert!(out.split('\n').any(|l| l == url));
        let table = "| a | b |\n|---|---|\n| a very long cell that would wrap if it were prose | x |";
        assert_eq!(answer_text(table, ON, 20), table);
        assert_eq!(answer_text("a\n\n\nb\n", ON, 80), "a\n\n\nb\n");
        assert_eq!(answer_text("one two three four five six", ON, 20), "one two three four\nfive six");
        assert_eq!(answer_text("a\r\nb\r\n", ON, 80), "a\nb\n");
        // two-line paragraphs stay two lines (the model chose those breaks)
        assert_eq!(answer_text("first line\nsecond line", ON, 80), "first line\nsecond line");
        // leading indentation of prose is kept, but never leaves no room
        assert_eq!(answer_text("  indented text", ON, 80), "  indented text");
        let deep = format!("{}text that is long enough to wrap around", " ".repeat(30));
        for l in answer_text(&deep, ON, 20).split('\n') {
            assert!(display_width(l) <= 20, "{l:?}");
        }
    }

    #[test]
    fn answer_styled_spans_survive_wrapping() {
        let out = answer_text("**alpha beta gamma delta** epsilon zeta", ON, 20);
        assert_eq!(strip_ansi(&out), "alpha beta gamma\ndelta epsilon zeta");
        // each word opens and closes its own bold, so a wrap never leaks a style
        assert_eq!(out.split('\n').next().unwrap(), "\x1b[1malpha\x1b[0m \x1b[1mbeta\x1b[0m \x1b[1mgamma\x1b[0m");
    }

    #[test]
    fn answer_prose_never_exceeds_the_width() {
        let samples = [
            "The quick brown fox jumps over the lazy dog and keeps running through the forest.",
            "日本語のテキストはスペースがないのでとても長い一つの単語として扱われます。これは長い。",
            "🚀🚀🚀 launch 🚀🚀 sequence 🚀 initiated 🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀",
            "supercalifragilisticexpialidocious_and_then_some_more_characters_without_any_space",
            "**bold supercalifragilisticexpialidocious_and_then_some_more_characters** `inlinecode_that_is_quite_long_indeed`",
            "- a list item with `code` and **bold** and a verylongwordthatdoesnotfitanywhereatall_really",
            "e\u{301}e\u{301}e\u{301} combining marks e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
            "# A heading that is long enough that it has to wrap onto another row of text",
            "> a quote that is long enough that it has to wrap onto another row of text",
        ];
        for t in samples {
            for w in [MIN_WIDTH, 21, 25, 33, 40, 80] {
                for l in answer_text(t, ON, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w} for {t:?}");
                }
            }
        }
    }

    #[test]
    fn answer_text_survives_weird_input() {
        let weird = [
            "",
            "\n\n\n",
            "```",
            "``````",
            "`",
            "``",
            "`a",
            "**",
            "*",
            "* ",
            "- ",
            "1.",
            "1. ",
            "#",
            "# ",
            "| ",
            "\t\t- tabbed",
            "\x1b[31mred\x1b[0m text",
            "\x1b",
            "\u{0}\u{7}\u{8}",
            "🚀`🚀**🚀`**",
            "**`**`**",
            &"`".repeat(500),
            &"*".repeat(501),
            &"a".repeat(10_000),
            &"- ".repeat(1_000),
            "e\u{301}",
            "\u{200d}\u{200d}",
            "```rust\n```rust\n```\n```",
            "> > > > >",
            "99999. not a list",
        ];
        for t in weird {
            for w in [0, 1, 20, 80] {
                let out = answer_text(t, ON, w);
                // model text can never drive the terminal: only our own SGR sequences remain
                let mut rest = out.as_str();
                while let Some(i) = rest.find('\x1b') {
                    let after = &rest[i..];
                    let end = after.find('m').expect("only SGR sequences");
                    assert!(after[1..end].chars().all(|c| c == '[' || c.is_ascii_digit() || c == ';'), "{out:?}");
                    rest = &after[end + 1..];
                }
                let _ = answer_text(t, OFF, w);
            }
        }
    }

    #[test]
    fn answer_text_strips_model_escapes() {
        let out = answer_text("hello \x1b[2Jworld \x1b]0;title\x07done", ON, 80);
        assert_eq!(out, "hello world done");
        assert_eq!(answer_text("```\n\x1b[31mred\n```", ON, 80), "    \x1b[2mred\x1b[0m");
    }

    #[test]
    fn colour_off_on_a_terminal_still_strips_control_sequences_but_pipes_stay_raw() {
        let hostile = "ok\x1b]0;pwned\x07 \x1b[2Jdone\x07";
        let tty_no_colour = Style::resolve(true, Some("1"), None, None);
        assert!(!tty_no_colour.enabled && tty_no_colour.terminal);
        let shown = answer_text(hostile, tty_no_colour, 80);
        assert!(!shown.contains('\x1b') && !shown.contains('\x07'), "{shown:?}");
        assert!(shown.contains("ok") && shown.contains("done"));
        let piped = Style::resolve(false, None, None, None);
        assert_eq!(answer_text(hostile, piped, 80), hostile, "non-terminal output is passed through byte for byte");
    }
}
