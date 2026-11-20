//! Pure renderers for the agent pillars: the workers panel, nested worker transcript,
//! result cards, timeline and conflict warnings ([`workers`]); the orchestration decision
//! card, plan, phase ribbon, review loop, savings meter and flow diagram ([`orch`]); and
//! the live task list ([`tasks`]).
//!
//! Same conventions as [`crate::tui::render`]: plain facts in, a `String` out (no trailing
//! newline), a disabled [`crate::tui::render::Style`] adds no escape bytes, and colour is never
//! the only cue (every state has a glyph AND a word). Text that came from a model or a
//! worker is cleaned to one plain line first.


use crate::tui::render::{strip_ansi, truncate_to_width, MIN_WIDTH};

/// A single plain line from untrusted text: escapes and control characters removed,
/// whitespace runs collapsed.
pub(crate) fn clean(s: &str) -> String {
    let plain: String = strip_ansi(s).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `s` cleaned and cut to `width` columns.
pub(crate) fn fit(s: &str, width: usize) -> String {
    truncate_to_width(&clean(s), width)
}

pub(crate) fn clamp_width(width: usize) -> usize {
    width.max(MIN_WIDTH)
}

/// A progress bar: `▰▰▰▱▱` for `done` of `total` in `cells` cells. Only a finished job
/// shows full, and a started one shows at least one cell.
pub fn bar(done: usize, total: usize, cells: usize) -> String {
    let filled = if total == 0 || done == 0 {
        0
    } else if done >= total {
        cells
    } else {
        (done * cells / total).clamp(1, cells.saturating_sub(1))
    };
    format!("{}{}", "▰".repeat(filled), "▱".repeat(cells - filled))
}

pub(crate) fn usd(v: f64) -> String {
    format!("${v:.2}")
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_shows_progress_without_lying_at_the_ends() {
        assert_eq!(bar(0, 5, 5), "▱▱▱▱▱");
        assert_eq!(bar(5, 5, 5), "▰▰▰▰▰");
        assert_eq!(bar(3, 5, 5), "▰▰▰▱▱");
        assert_eq!(bar(4, 5, 5), "▰▰▰▰▱");
        assert_eq!(bar(99, 100, 5), "▰▰▰▰▱", "one short of done never shows full");
        assert_eq!(bar(1, 100, 5), "▰▱▱▱▱", "started shows at least one cell");
        assert_eq!(bar(0, 0, 3), "▱▱▱");
    }

    #[test]
    fn clean_strips_escapes_and_newlines() {
        assert_eq!(clean("a\x1b[31mb\x1b[0m\n\tc"), "ab c");
    }
}
