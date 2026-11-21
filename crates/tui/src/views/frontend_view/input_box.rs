//! The input box, its edge label and the key hints under it.

use std::time::Instant;

use super::layout::{boxed_in, bottom_edge, frame};
use super::theme::Theme;
use super::INPUT_ROWS;
use crate::app::screen_state::App;
use crate::render::{display_width, truncate_to_width};

/// The input box and where the cursor is in its rows. Its bottom edge carries the mode.
pub(super) fn input_box(app: &App, w: usize, th: &Theme) -> (Vec<String>, (usize, usize)) {
    // Inside the borders: `│ ❯ ` before the text, ` │` after.
    let text_w = w.saturating_sub(6).max(1);
    let edge = |s: &str| th.field(s);
    let mut rows = vec![frame('╭', '╮', "", w, th, edge)];
    let label = box_label(app, th);
    let ghost = app.ghost();
    if app.editor.is_empty() {
        // A suggested next prompt takes the placeholder's place; the edge says how to take it.
        let (shown, hint) = match &ghost {
            Some(suggestion) => (truncate_to_width(suggestion, text_w), "→ to use"),
            None if app.working.is_some() => ("Type to queue a message for after this turn".to_string(), ""),
            None => ("Ask anything · / for commands · @ for files".to_string(), ""),
        };
        rows.push(boxed_in(&format!("{} {}", th.accent("❯"), th.muted(&shown)), w, edge));
        rows.push(bottom_edge(hint, &label, w, th));
        return (rows, (1, 4));
    }
    let (lines, (cr, cc)) = app.editor.layout(text_w);
    let first = cr.saturating_sub(INPUT_ROWS - 1);
    for (i, line) in lines.iter().enumerate().skip(first).take(INPUT_ROWS) {
        let lead = if i == 0 { th.accent("❯") } else { " ".to_string() };
        // The ghost text follows the cursor, on its row, cut to what is left of it.
        let rest = match &ghost {
            Some(g) if i == cr => truncate_to_width(g, text_w.saturating_sub(display_width(line))),
            _ => String::new(),
        };
        let rest = if rest.is_empty() { rest } else { th.muted(&rest) };
        rows.push(boxed_in(&format!("{lead} {line}{rest}"), w, edge));
    }
    let hidden = lines.len().saturating_sub(first + INPUT_ROWS);
    let hint = if first > 0 || hidden > 0 { format!("{} lines", lines.len()) } else { String::new() };
    rows.push(bottom_edge(&hint, &label, w, th));
    (rows, (1 + cr - first, 4 + cc))
}

/// What the box's edge says: `NORMAL · auto · edit mode`, the mode in its colour.
fn box_label(app: &App, th: &Theme) -> String {
    let mut parts = Vec::new();
    if let Some(label) = app.vim.as_ref().and_then(|v| v.label()) {
        parts.push(th.strong(label));
    }
    parts.push(th.muted(&app.route));
    parts.push(th.mode(app.mode));
    parts.join(&th.muted(" · "))
}

/// The key hints under the box, for what the keys do right now; a notice takes their place
/// for a moment.
pub(super) fn footer(app: &App, w: usize, now: Instant, th: &Theme) -> String {
    if let Some((text, _)) = app.notice.as_ref().filter(|(_, at)| now.duration_since(*at) < crate::app::screen_state::NOTICE_FOR) {
        return truncate_to_width(&format!("  {}", th.warning(text)), w);
    }
    let keys: &[(&str, &str)] = if app.menu.is_some() {
        &[("↑↓", "select"), ("Tab", "complete"), ("Enter", "run"), ("Esc", "close")]
    } else if app.working.is_some() {
        // The working row above already says `esc to stop`.
        &[("Enter", "queue"), ("Shift+Enter", "newline"), ("Shift+Tab", "mode")]
    } else {
        &[("Enter", "send"), ("Shift+Enter", "newline"), ("Shift+Tab", "mode"), ("/", "commands"), ("@", "files")]
    };
    th.keys(keys, w)
}
