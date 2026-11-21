//! Assistant text with light markdown for a styled terminal.

use crate::tui::render::highlight::{Lang, LineState};
use crate::tui::render::style::Style;
use crate::tui::render::width::{clamp_width, display_width};
use crate::tui::render::wrap::Span;

mod block;
mod inline;
mod table;
#[cfg(test)]
mod tests;

use block::{code_line, expand_leading_tabs, fence_of, heading_text, list_marker, sanitize_answer_line, Fence};
use inline::{draw_wrapped, parse_inline, to_words};
use table::render_table;

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
