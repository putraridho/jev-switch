//! What the screen looks like around the conversation: the header row (where you are, what the
//! router has), the working row, queued messages, the `/` menu, the input box (or an approval
//! card in its place) and the key hints under it, as rows of styled text and the cursor's
//! place. Pure: every screen is a snapshot test at any width, with and without colour.
//!
//! The look follows one rule: only the input box is boxed. Everything else is a row of text,
//! set apart by space, weight and a shaded band, so the conversation keeps the room.
//!
//! Every row is cut to `width - 1` columns, so no row ever wraps (the screen erases the live
//! area by counting rows).

mod activity;
mod approval;
mod chat;
mod header;
mod input_box;
mod layout;
mod menu;
mod theme;
#[cfg(test)]
mod tests;

use std::time::Instant;

use crate::app::screen_state::App;
use crate::render::{display_width, truncate_to_width, Style};
use approval::{approval_card, choice};
use chat::chat_rows;
use header::header_row;
use input_box::{footer, input_box};
use layout::{raised_row, spread};
use menu::menu_rows;
use theme::Theme;
pub use activity::{doing, working_line};
use activity::workers_rows;

/// The most rows of typed text the box shows; longer text scrolls with the cursor.
pub const INPUT_ROWS: usize = 8;
/// The most rows of a tool's input the approval card shows at once.
pub const CARD_INPUT_ROWS: usize = 12;

/// One drawn live area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rows: Vec<String>,
    /// Row and column of the text cursor; `None` hides it.
    pub cursor: Option<(usize, usize)>,
}

/// Where the live area is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Under the conversation in the terminal's own scrollback: the header rides on top of it.
    Inline,
    /// Full screen: the header is pinned to the top row instead.
    Full,
}

/// Draw the live area for `width` terminal columns at time `now`.
pub fn render(app: &App, width: usize, now: Instant, style: Style) -> Frame {
    live(app, width, now, style, Place::Inline)
}

fn live(app: &App, width: usize, now: Instant, style: Style, place: Place) -> Frame {
    let th = Theme(style);
    // One column short of the terminal, so a full row never wraps.
    let w = width.saturating_sub(1).max(20);
    let mut rows: Vec<String> = Vec::new();
    let mut cursor = None;
    if place == Place::Inline {
        rows.push(header_row(app, w, &th));
    }

    // The working row steps aside while a question waits: one thing to look at.
    if let Some(work) = app.working.as_ref().filter(|_| app.approval.is_none()) {
        let hint = "esc to stop";
        let elapsed = now.duration_since(work.started);
        // Who is working goes first when the row is short: what runs and how to stop it stay.
        let who = if work.label == "thinking" { String::new() } else { format!("{} · ", work.label) };
        let full = display_width(&working_line(work, elapsed, usize::MAX, style)) + 2;
        let right = if full + display_width(&who) + display_width(hint) + 2 <= w { format!("{who}{hint}") } else { hint.to_string() };
        let line = working_line(work, elapsed, w.saturating_sub(display_width(&right) + 4), style);
        rows.push(spread(&format!("  {line}"), &th.muted(&right), w));
    }
    rows.extend(workers_rows(app, w, style));
    for (i, queued) in app.queue.iter().take(3).enumerate() {
        let first = queued.lines().next().unwrap_or_default();
        let more = if queued.lines().count() > 1 { " …" } else { "" };
        rows.push(truncate_to_width(&format!("  {} {first}{more}", th.muted(&format!("#{}", i + 1))), w));
    }
    if app.queue.len() > 3 {
        rows.push(th.muted(&format!("  … and {} more queued", app.queue.len() - 3)));
    }

    if let Some(c) = &app.confirm {
        rows.push(raised_row(&th.strong(&c.question), w, &th));
        rows.push(raised_row("", w, &th));
        for (i, label) in ["Yes", "No"].iter().enumerate() {
            rows.push(raised_row(&choice(i, label, i == c.selected, &th), w, &th));
        }
        rows.push(th.keys(&[("y", "yes"), ("n/Esc", "no"), ("↑↓", "move"), ("Enter", "choose")], w));
    } else if let Some(a) = &app.approval {
        rows.extend(approval_card(a, w, &th));
    } else {
        if let Some(m) = &app.menu {
            rows.extend(menu_rows(m, w, &th));
        }
        let (box_rows, at) = input_box(app, w, &th);
        cursor = Some((rows.len() + at.0, at.1));
        rows.extend(box_rows);
        rows.push(footer(app, w, now, &th));
    }
    Frame { rows, cursor }
}

/// The whole terminal, `cols` by `height`: the header pinned on top, the chat under it, and
/// the input box with its key hints at the bottom. Returns the frame (exactly `height` rows)
/// and how many rows the chat pane has.
pub fn render_full(app: &App, cols: usize, height: usize, now: Instant, style: Style) -> (Frame, usize) {
    let w = cols.saturating_sub(1).max(20);
    let mut live = live(app, cols, now, style, Place::Full);
    // A terminal too short for the live area keeps its bottom rows (the box and the keys).
    if live.rows.len() > height {
        let cut = live.rows.len() - height;
        live.rows.drain(..cut);
        live.cursor = live.cursor.and_then(|(r, c)| r.checked_sub(cut).map(|r| (r, c)));
    }
    // The header takes the top row once there is a row to spare for it.
    let header = usize::from(height > live.rows.len());
    let chat_h = height - live.rows.len() - header;
    let mut rows = Vec::with_capacity(height);
    if header == 1 {
        rows.push(header_row(app, w, &Theme(style)));
    }
    rows.extend(chat_rows(app, w, chat_h, style));
    let top = rows.len();
    rows.extend(live.rows);
    let cursor = live.cursor.map(|(r, c)| (r + top, c));
    (Frame { rows, cursor }, chat_h)
}
