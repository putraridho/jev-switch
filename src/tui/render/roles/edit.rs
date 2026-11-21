//! An edit as a small diff under its row.

use super::tool::plural;
use crate::tui::render::ansi::clean_text;
use crate::tui::render::style::Style;
use crate::tui::render::width::{clamp_width, display_width, truncate_to_width};

/// The most lines of an edit shown under it; the rest are counted.
pub const EDIT_PREVIEW_LINES: usize = 6;

/// The lines an edit takes out (red, `-`) and puts in (green, `+`), at most
/// [`EDIT_PREVIEW_LINES`], the rest counted; a whole new file says how long it is.
pub fn edit_preview(removed: &[String], added: &[String], style: Style, width: usize) -> String {
    edit_preview_at(removed, added, None, style, width)
}

/// [`edit_preview`] with the line the change starts at: each row then carries its line number
/// in a gutter, and with colour the rows are tinted (a muted red and green band), so the
/// change reads as a diff at a glance. The `-`/`+` stay, so it reads without colour too.
pub fn edit_preview_at(removed: &[String], added: &[String], start: Option<usize>, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut rows = Vec::new();
    if removed.is_empty() {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("new file, {}", plural(added.len(), "line", "lines")))));
    } else {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("+{} −{}", added.len(), removed.len()))));
    }
    let gutter_w = start.map_or(0, |s| (s + removed.len().max(added.len())).to_string().len());
    let numbered = |n: usize| if gutter_w == 0 { String::new() } else { style.dim(&format!("{:>gutter_w$} ", n)) };
    let shown: Vec<(bool, String, usize)> = removed
        .iter()
        .enumerate()
        .map(|(i, l)| (false, format!("- {}", clean_text(l)), i))
        .chain(added.iter().enumerate().map(|(i, l)| (true, format!("+ {}", clean_text(l)), i)))
        .collect();
    let total = shown.len();
    for (is_added, text, i) in shown.into_iter().take(EDIT_PREVIEW_LINES) {
        let n = numbered(start.unwrap_or(0) + i);
        // The tint runs the width of the row, so the change is one band, not ragged text.
        let body = truncate_to_width(&text, width.saturating_sub(6 + if gutter_w == 0 { 0 } else { gutter_w + 1 }));
        let pad = if style.enabled { " ".repeat(width.saturating_sub(7 + if gutter_w == 0 { 0 } else { gutter_w + 1 }).saturating_sub(display_width(&body))) } else { String::new() };
        let colored = if is_added { style.green(&body) } else { style.red(&body) };
        let band = style.tint(is_added, &format!("{colored}{pad}"));
        rows.push(format!("      {n}{band}"));
    }
    if total > EDIT_PREVIEW_LINES {
        rows.push(style.dim(&format!("      … {} more lines · /diff after the turn", total - EDIT_PREVIEW_LINES)));
    }
    rows.join("\n")
}
