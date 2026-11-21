//! Line-level block syntax: headings, list items, fences and code lines.

use crate::render::ansi::strip_ansi;
use crate::render::highlight::{highlight_line, Lang, LineState};
use crate::render::style::Style;

/// Remove escape sequences and control characters (except tab) from model text, so
/// nothing the model prints can drive the terminal.
pub(super) fn sanitize_answer_line(line: &str) -> String {
    strip_ansi(line.trim_end_matches('\r')).chars().filter(|c| !c.is_control() || *c == '\t').collect()
}

pub(super) fn expand_leading_tabs(line: &str) -> String {
    let rest = line.trim_start_matches('\t');
    format!("{}{rest}", "    ".repeat(line.len() - rest.len()))
}

/// `(indent, marker, rest)` of a list item or quote line.
pub(super) fn list_marker(line: &str) -> Option<(usize, &str, &str)> {
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
pub(super) fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start_matches(' ');
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    let info = t[n..].trim();
    (n >= 3 && !(c == '`' && info.contains('`'))).then_some((c, n, info))
}

/// The text of a `# heading` line (1 to 6 hashes then a space), if it is one.
pub(super) fn heading_text(trimmed: &str) -> Option<&str> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    let text = rest.trim().trim_end_matches('#').trim_end();
    ((1..=6).contains(&hashes) && !text.is_empty()).then_some(text)
}

/// An open code fence: what closes it, and how its lines are coloured.
pub(super) struct Fence {
    pub(super) ch: char,
    pub(super) len: usize,
    pub(super) lang: Option<Lang>,
    pub(super) state: LineState,
}

/// One line of a code block, indented four columns. A known language is highlighted;
/// any other is dimmed.
pub(super) fn code_line(line: &str, fence: &mut Fence, style: Style) -> String {
    let line = line.trim_end();
    let body = match fence.lang {
        Some(lang) => highlight_line(lang, line, &mut fence.state, style),
        None => style.dim(line),
    };
    format!("    {body}")
}
