//! What you sent, and the row that says where a turn went.

use crate::tui::render::ansi::strip_ansi;
use crate::tui::render::route::{short_reason, RouteFacts};
use crate::tui::render::style::{Style, Theme};
use crate::tui::render::width::{clamp_width, display_width, truncate_to_width};
use crate::tui::render::wrap::wrap_to_width;

/// The message you sent, as it stays in the scrollback: with colour, a full-width shaded
/// band, a blank band row above and below the text, `❯` on its first row and the time it was
/// sent (`stamp`, `6:27 PM`) flush right; the one block of background in the scrollback, so
/// your prompts are easy to find. Without colour, `❯ text` under a rule between turns.
/// `first` leaves out the separation above.
pub fn sent_block(text: &str, first: bool, stamp: Option<&str>, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let row_w = width.saturating_sub(1);
    let mut out = Vec::new();
    // With colour the band's own blank top row sets it apart; without, a rule does.
    if !first && !style.enabled {
        out.push("─".repeat(row_w));
    }
    let stamp = stamp.filter(|_| style.enabled).unwrap_or("");
    // Text wraps clear of the stamp: `  ❯ ` before it, `  6:27 PM  ` after.
    let right = if stamp.is_empty() { 2 } else { display_width(stamp) + 4 };
    let blank = style.band(&" ".repeat(row_w));
    if style.enabled {
        out.push(blank.clone());
    }
    let mut first_row = true;
    for line in text.split('\n') {
        for part in wrap_to_width(line, row_w.saturating_sub(4 + right).max(1)) {
            let lead = if first_row { "❯" } else { " " };
            if style.enabled {
                let tail = if first_row && !stamp.is_empty() { format!("{}  ", style.dim(stamp)) } else { String::new() };
                let fill = row_w.saturating_sub(4 + display_width(&part) + display_width(&strip_ansi(&tail)));
                out.push(style.band(&format!("  {} {}{}{tail}", style.accent(lead), style.bold(&part), " ".repeat(fill))));
            } else {
                out.push(format!("{lead} {part}"));
            }
            first_row = false;
        }
    }
    if style.enabled {
        out.push(blank);
    }
    out.push(String::new());
    out.join("\n")
}

/// Where a turn went: `  ⇄ claude · sonnet-5-5   feature · switched from codex (limit)`.
/// The provider in its colour, the router's reason in a few words. This is the router at
/// work, so it leads every turn; `⇄` is the router's mark wherever it shows.
pub fn route_row(f: &RouteFacts, style: Style, width: usize) -> String {
    let th = Theme::new(style);
    let who = if f.model.is_empty() { th.name(&f.provider) } else { format!("{} {}", th.name(&f.provider), style.dim(&format!("· {}", f.model))) };
    let why = short_reason(&f.reason);
    let why = if why.is_empty() { f.task.clone() } else { why };
    let mark = style.dim("⇄");
    let line = if why.is_empty() { format!("  {mark} {who}") } else { format!("  {mark} {who}   {}", style.dim(&why)) };
    truncate_to_width(&line, clamp_width(width))
}
