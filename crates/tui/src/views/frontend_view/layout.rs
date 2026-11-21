//! The row shapes the screen is built from: spread rows, box edges, boxed and raised rows.

use super::theme::Theme;
use crate::render::{display_width, truncate_to_width};

/// `left` and `right` on one row of `width` columns, `right` flush right. When both do not
/// fit, `left` is cut to make room, down to a few columns; past that `right` gives way.
pub(super) fn spread(left: &str, right: &str, width: usize) -> String {
    let (lw, rw) = (display_width(left), display_width(right));
    if right.is_empty() {
        return truncate_to_width(left, width);
    }
    if lw + rw + 2 > width {
        let room = width.saturating_sub(rw + 2);
        if room < 12 {
            return truncate_to_width(left, width);
        }
        let cut = truncate_to_width(left, room);
        return format!("{cut}{}{right}", " ".repeat(width - display_width(&cut) - rw));
    }
    format!("{left}{}{right}", " ".repeat(width - lw - rw))
}

/// A box border row: `╭─ title ─────╮` (or the bottom one), its lines in `edge`.
pub(super) fn frame(left: char, right: char, title: &str, width: usize, th: &Theme, edge: impl Fn(&str) -> String) -> String {
    let inner = width.saturating_sub(2);
    if title.is_empty() {
        return edge(&format!("{left}{}{right}", "─".repeat(inner)));
    }
    let shown = truncate_to_width(title, inner.saturating_sub(4));
    let fill = inner.saturating_sub(display_width(&shown) + 3);
    format!("{}{}{}", edge(&format!("{left}─ ")), th.strong(&shown), edge(&format!(" {}{right}", "─".repeat(fill))))
}

/// The input box's bottom edge: a hint set in on the left (`→ to use`, `12 lines`) and the
/// route and mode on the right, `╰─ → to use ──────── auto · edit mode ─╯`.
pub(super) fn bottom_edge(hint: &str, label: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(2);
    let label_w = display_width(label);
    let left = if hint.is_empty() { String::new() } else { format!("{}{} ", th.field("─ "), th.muted(hint)) };
    let left_w = display_width(&crate::render::strip_ansi(&left));
    // Too narrow for both: the label goes, the hint stays.
    if left_w + label_w + 5 > inner {
        let fill = inner.saturating_sub(left_w);
        return format!("{}{left}{}", th.field("╰"), th.field(&format!("{}╯", "─".repeat(fill))));
    }
    let fill = inner - left_w - label_w - 3;
    format!("{}{left}{}{label}{}", th.field("╰"), th.field(&format!("{} ", "─".repeat(fill))), th.field(" ─╯"))
}

/// A row inside a box: `│ content   │`, content cut or padded to fit, its sides in `edge`.
pub(super) fn boxed_in(content: &str, width: usize, edge: impl Fn(&str) -> String) -> String {
    let inner = width.saturating_sub(4);
    let shown = truncate_to_width(content, inner);
    let pad = inner.saturating_sub(display_width(&shown));
    format!("{} {shown}{} {}", edge("│"), " ".repeat(pad), edge("│"))
}

/// A row on a raised surface: shaded the whole `width`, an accent bar `▌` on its left edge.
pub(super) fn raised_row(content: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(4);
    let shown = truncate_to_width(content, inner);
    let pad = inner.saturating_sub(display_width(&shown));
    th.raised(&format!("{} {shown}{}  ", th.accent("▌"), " ".repeat(pad)))
}
