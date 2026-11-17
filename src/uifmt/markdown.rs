//! Assistant text with light markdown for a styled terminal.

use super::ansi::strip_ansi;
use super::highlight::{highlight_line, Lang, LineState};
use super::style::Style;
use super::width::{clamp_width, display_width};
use super::wrap::{push_char, wrap_words, Span, Tok, Word};

/// `s` painted the way `span` is drawn.
fn paint_span(style: Style, span: Span, s: &str) -> String {
    match span {
        Span::Plain => s.to_string(),
        Span::Bold => style.bold(s),
        Span::Code => style.cyan(s),
        Span::Dim => style.dim(s),
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
    let mut fence: Option<Fence> = None;
    let mut table: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = expand_leading_tabs(&sanitize_answer_line(raw));
        if let Some(open) = fence.as_mut() {
            match fence_of(&line) {
                Some((c, n, info)) if c == open.ch && n >= open.len && info.is_empty() => fence = None,
                _ if line.trim().is_empty() => out.push(String::new()),
                _ => out.push(code_line(&line, open, style)),
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('|') {
            table.push(line);
            continue;
        }
        out.extend(render_table(&std::mem::take(&mut table), style, width));
        if let Some((c, n, info)) = fence_of(&line) {
            let lang = info.split_whitespace().next();
            fence = Some(Fence { ch: c, len: n, lang: lang.and_then(Lang::from_info), state: LineState::Normal });
            if let Some(lang) = lang {
                out.push(format!("  {}", style.dim(lang)));
            }
            continue;
        }
        if trimmed.is_empty() {
            out.push(String::new());
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
    out.extend(render_table(&table, style, width));
    out.join("\n")
}

/// An open code fence: what closes it, and how its lines are coloured.
struct Fence {
    ch: char,
    len: usize,
    lang: Option<Lang>,
    state: LineState,
}

/// One line of a code block, indented four columns. A known language is highlighted;
/// any other is dimmed.
fn code_line(line: &str, fence: &mut Fence, style: Style) -> String {
    let line = line.trim_end();
    let body = match fence.lang {
        Some(lang) => highlight_line(lang, line, &mut fence.state, style),
        None => style.dim(line),
    };
    format!("    {body}")
}

/// How a table column is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

/// The cells of a `| a | b |` row, trimmed. A `\|` is part of a cell.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with("\\|") { t } else { t.strip_suffix('|').unwrap_or(t) };
    let (mut cells, mut cur, mut escaped) = (Vec::new(), String::new(), false);
    for c in t.chars() {
        if c == '|' && !escaped {
            cells.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    cells.push(cur.trim().to_string());
    cells
}

/// The alignment a separator cell (`---`, `:--`, `--:`, `:-:`) asks for.
fn parse_align(cell: &str) -> Option<Align> {
    let dashes = cell.trim_matches(':');
    if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
        return None;
    }
    Some(match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// `cell` padded with spaces to `width` columns by `align`, painted by `paint`.
fn pad_cell(cell: &str, width: usize, align: Align, paint: impl Fn(&str) -> String) -> String {
    let gap = width.saturating_sub(display_width(cell));
    let (left, right) = match align {
        Align::Left => (0, gap),
        Align::Right => (gap, 0),
        Align::Center => (gap / 2, gap - gap / 2),
    };
    format!("{}{}{}", " ".repeat(left), paint(cell), " ".repeat(right))
}

/// A table row's cells drawn in columns, with no trailing spaces.
fn draw_row(cells: &[String], widths: &[usize], aligns: &[Align], paint: impl Fn(&str) -> String) -> String {
    let empty = String::new();
    let drawn: Vec<String> =
        (0..widths.len()).map(|i| pad_cell(cells.get(i).unwrap_or(&empty), widths[i], aligns[i], &paint)).collect();
    drawn.join("  ").trim_end().to_string()
}

/// A GitHub pipe table (header, separator, rows) drawn as aligned columns with a dim
/// rule under the header. Anything that is not a table, or is wider than `width`, is
/// returned as the lines were written.
fn render_table(lines: &[String], style: Style, width: usize) -> Vec<String> {
    let as_written = || lines.iter().map(|l| l.trim_end().to_string()).collect();
    let rows: Vec<Vec<String>> = lines.iter().map(|l| split_row(l)).collect();
    let Some(aligns) = rows.get(1).and_then(|r| r.iter().map(|c| parse_align(c)).collect::<Option<Vec<Align>>>()) else {
        return as_written();
    };
    let columns = aligns.len();
    let body: Vec<&Vec<String>> = rows.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, r)| r).collect();
    if rows[0].len() != columns || body.iter().any(|r| r.len() > columns) {
        return as_written();
    }
    let widths: Vec<usize> =
        (0..columns).map(|i| body.iter().filter_map(|r| r.get(i)).map(|c| display_width(c)).max().unwrap_or(0).max(1)).collect();
    if widths.iter().sum::<usize>() + 2 * (columns - 1) > width {
        return as_written();
    }
    let rule = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("  ");
    let mut out = vec![draw_row(&rows[0], &widths, &aligns, |c| style.bold(c)), style.dim(&rule)];
    out.extend(body.iter().skip(1).map(|r| draw_row(r, &widths, &aligns, |c| c.to_string())));
    out
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
        let t = "before\n```text\nfn main() {\n\n    let x = 1;\n}\n```\nafter";
        assert_eq!(
            answer_text(t, ON, 80),
            "before\n  \x1b[2mtext\x1b[0m\n    \x1b[2mfn main() {\x1b[0m\n\n    \x1b[2m    let x = 1;\x1b[0m\n    \x1b[2m}\x1b[0m\nafter"
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
    fn known_languages_are_highlighted_and_keep_their_layout() {
        let t = "```rust\n    let s = \"hi\";\n\nfn f() {}\n```";
        let out = answer_text(t, ON, 80);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(strip_ansi(&out), "  rust\n        let s = \"hi\";\n\n    fn f() {}");
        assert!(lines[1].contains("\x1b[35mlet\x1b[0m") && lines[1].contains("\x1b[32m\"hi\"\x1b[0m"), "{out:?}");
        assert!(lines[3].contains("\x1b[35mfn\x1b[0m"), "{out:?}");
        // a long code line is still not wrapped
        let long = format!("```rust\nlet x = \"{}\";\n```", "word ".repeat(40));
        assert_eq!(answer_text(&long, ON, 30).matches('\n').count(), 1);
    }

    #[test]
    fn diff_blocks_colour_added_and_removed_lines() {
        let out = answer_text("```diff\n@@ -1 +1 @@\n-old\n+new\n ctx\n```", ON, 80);
        assert!(out.contains("\x1b[36m@@ -1 +1 @@\x1b[0m"), "{out:?}");
        assert!(out.contains("    \x1b[31m-old\x1b[0m"), "{out:?}");
        assert!(out.contains("    \x1b[32m+new\x1b[0m"), "{out:?}");
    }

    #[test]
    fn highlighted_blocks_have_no_escapes_with_style_off() {
        let t = "```rust\nfn main() { let s = \"x\"; }\n```\n```diff\n+a\n-b\n```\n| a | b |\n|---|---|\n| 1 | 2 |\n[t](http://u)";
        assert_eq!(answer_text(t, OFF, 80), t);
        let tty_plain = Style::resolve(true, Some("1"), None, None);
        no_esc(&answer_text(t, tty_plain, 80));
    }

    #[test]
    fn tables_are_aligned_in_columns() {
        let t = "| name | n |\n|:--|--:|\n| alpha | 1 |\n| b | 100 |";
        let out = answer_text(t, ON, 80);
        assert_eq!(strip_ansi(&out), "name     n\n─────  ───\nalpha    1\nb      100");
        assert!(out.starts_with("\x1b[1mname\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[2m─────  ───\x1b[0m"), "{out:?}");
        // wide characters count by columns, centre alignment splits the gap
        let wide = answer_text("| a | b |\n|:-:|---|\n| 日本 | x |", ON, 80);
        assert_eq!(strip_ansi(&wide), " a    b\n────  ─\n日本  x");
        // prose around a table is kept, and a short row is padded
        let mixed = answer_text("before\n| a | b |\n|---|---|\n| 1 |\nafter", ON, 80);
        assert_eq!(strip_ansi(&mixed), "before\na  b\n─  ─\n1\nafter");
    }

    #[test]
    fn tables_too_wide_or_malformed_are_left_alone() {
        let wide = "| a | b |\n|---|---|\n| a very long cell that does not fit | x |";
        assert_eq!(answer_text(wide, ON, 20), wide);
        for t in ["| a | b |", "| a | b |\n| 1 | 2 |", "| a | b |\n|---|\n| 1 | 2 |", "| a |\n|---|\n| 1 | 2 | 3 |", "|\n|"] {
            assert_eq!(answer_text(t, ON, 80), t, "{t:?}");
        }
    }

    #[test]
    fn links_show_the_text_then_a_dim_url() {
        assert_eq!(answer_text("see [docs](https://a.io/x) now", ON, 80), "see docs \x1b[2m(https://a.io/x)\x1b[0m now");
        assert_eq!(answer_text("[https://a.io](https://a.io)", ON, 80), "\x1b[2mhttps://a.io\x1b[0m");
        assert_eq!(strip_ansi(&answer_text("**[b](http://u)**.", ON, 80)), "b (http://u).");
        // not links: no target, spaces in the target, empty parts, nested brackets
        for t in ["[a] (b)", "[a](b c)", "[](x)", "[a]()", "[a [b](c)", "[a](b"] {
            let out = strip_ansi(&answer_text(t, ON, 80));
            assert!(out.contains('[') || out == "a [b] (c)" || t == "[a [b](c)", "{t:?} -> {out:?}");
        }
        assert_eq!(answer_text("[a](b c)", ON, 80), "[a](b c)");
    }

    #[test]
    fn link_urls_are_never_broken() {
        let url = "https://example.com/some/really/long/path/that/never/fits/on/one/line";
        let out = answer_text(&format!("read [the guide]({url}) for more details"), ON, 30);
        assert!(out.contains(&format!("\x1b[2m({url})\x1b[0m")), "{out:?}");
        // a target without a scheme is not cut either
        let path = "docs/a/very/long/relative/path/to/a/file.md";
        let out = answer_text(&format!("[x]({path})"), ON, 20);
        assert!(strip_ansi(&out).contains(&format!("({path})")), "{out:?}");
    }

    #[test]
    fn answer_unclosed_fence_swallows_the_rest_as_code() {
        let out = answer_text("```txt\nprint(1)\n# still code\n**not bold**", ON, 80);
        assert_eq!(
            out,
            "  \x1b[2mtxt\x1b[0m\n    \x1b[2mprint(1)\x1b[0m\n    \x1b[2m# still code\x1b[0m\n    \x1b[2m**not bold**\x1b[0m"
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
