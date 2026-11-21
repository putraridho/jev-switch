//! The raised cards that take the input box's place: a tool's approval and yes/no questions.

use super::layout::raised_row;
use super::theme::Theme;
use super::CARD_INPUT_ROWS;
use crate::render::wrap_to_width;

/// One numbered choice on a card: `● 1. Yes` when selected, `○ 2. No` otherwise.
pub(super) fn choice(i: usize, label: &str, selected: bool, th: &Theme) -> String {
    let text = format!("{}. {label}", i + 1);
    if selected {
        format!("{} {}", th.accent("●"), th.selected(&text))
    } else {
        format!("{} {}", th.muted("○"), th.muted(&text))
    }
}

/// A tool waiting for an answer, in place of the box: a raised card with the question, the
/// tool's input, the numbered choices, and the keys under it.
pub(super) fn approval_card(a: &crate::app::screen_state::Approval, w: usize, th: &Theme) -> Vec<String> {
    let inner = w.saturating_sub(4).max(1);
    let mut rows = vec![raised_row(&th.strong(&format!("Allow {}?", a.tool)), w, th)];
    let mut body: Vec<String> = Vec::new();
    for line in &a.lines {
        body.extend(wrap_to_width(line, inner));
    }
    let first = a.scroll.min(body.len().saturating_sub(1));
    for line in body.iter().skip(first).take(CARD_INPUT_ROWS) {
        rows.push(raised_row(line, w, th));
    }
    let below = body.len().saturating_sub(first + CARD_INPUT_ROWS);
    if first > 0 || below > 0 {
        rows.push(raised_row(&th.muted(&format!("… {} more lines · pgup/pgdn to scroll", first + below)), w, th));
    }
    for note in &a.notes {
        rows.push(raised_row(&th.muted(note), w, th));
    }
    rows.push(raised_row("", w, th));
    for (i, (_, label)) in a.choices().iter().enumerate() {
        rows.push(raised_row(&choice(i, label, i == a.selected, th), w, th));
    }
    if let Some(note) = &a.note {
        for line in wrap_to_width(note, w.saturating_sub(2)) {
            rows.push(format!("  {}", th.warning(&line)));
        }
    }
    let n = a.choices().len();
    let range = format!("1–{n}");
    rows.push(th.keys(&[(&range, "choose"), ("↑↓", "move"), ("Enter", "confirm"), ("Esc", "no"), ("Ctrl+C", "stop the turn")], w));
    rows
}
