//! Owns the session header and the lines offering the previous session, plus the one-time
//! first-run hint. Must not decide when to print them.

use std::path::Path;

use crate::doctor::domain::diagnose as doctor;
use crate::providers::domain::availability::Availability;
use crate::session::domain::saved_session::SessionInfo;
use crate::tui::render as uifmt;
use crate::tui::render::{AvailRow, ProviderStatus, Style};

/// `dir` with the home directory shown as `~`.
pub fn display_path(dir: &Path, home: Option<&str>) -> String {
    let full = dir.to_string_lossy();
    match home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty()) {
        Some(h) if full == h => "~".to_string(),
        Some(h) => full.strip_prefix(&format!("{h}/")).map_or_else(|| full.to_string(), |rest| format!("~/{rest}")),
        None => full.to_string(),
    }
}

/// The session header, one line: `jev-code 0.2.0 · claude, codex ready · ~/proj`. Providers
/// are named in words (no symbol to learn); the full table lives behind `/providers`, and
/// with nothing ready the line says where to look.
pub fn banner(dir: &Path, home: Option<&str>, routing_on: bool, rows: &[AvailRow], style: Style, width: usize) -> String {
    let width = width.max(uifmt::MIN_WIDTH);
    let ready: Vec<&str> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none()).map(|r| r.provider.as_str()).collect();
    let cooling: Vec<&str> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_some()).map(|r| r.provider.as_str()).collect();
    let mut providers = if ready.is_empty() { style.yellow("no provider ready · /doctor") } else { format!("{} ready", ready.join(", ")) };
    if !cooling.is_empty() {
        providers.push_str(&style.dim(&format!(" · {} cooling down", cooling.join(", "))));
    }
    let routing = if routing_on { String::new() } else { format!(" {} {}", style.dim("·"), style.yellow("Claude only")) };
    let brand = format!("{} {}", style.bold(&style.accent("jev-code")), style.dim(env!("CARGO_PKG_VERSION")));
    let sep = style.dim("·");
    // What can run comes before the folder: a long path is what gets cut, never the providers.
    let head = format!("{brand} {sep} {providers}{routing} {sep} ");
    let room = width.saturating_sub(uifmt::display_width(&head));
    uifmt::truncate_to_width(&format!("{head}{}", keep_tail(&display_path(dir, home), room)), width)
}

/// `path` cut from the front to `width` columns, `…/project/src`: the end of a path is the part
/// that names the folder, so that is what stays. Cut at a `/` when the last folder fits whole.
pub fn keep_tail(path: &str, width: usize) -> String {
    if uifmt::display_width(path) <= width {
        return path.to_string();
    }
    let mut tail = Vec::new();
    let mut used = 1; // the `…`
    for c in path.chars().rev() {
        used += uifmt::char_width(c);
        if used > width {
            break;
        }
        tail.push(c);
    }
    let tail: String = tail.into_iter().rev().collect();
    let tail = match tail.find('/') {
        Some(i) if i > 0 => &tail[i..],
        _ => tail.as_str(),
    };
    format!("…{tail}")
}

/// "just now", "5m ago", "2h ago", "3d ago".
pub fn human_ago(ms: u64) -> String {
    const MIN: u64 = 60_000;
    match ms / MIN {
        0 => "just now".to_string(),
        m @ 1..=59 => format!("{m}m ago"),
        m if m < 48 * 60 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (24 * 60)),
    }
}

/// The dim line offering the saved session: `previous session: 12 turns, 2h ago · /resume`.
pub fn previous_session_line(info: &SessionInfo, now_ms: u64, style: Style) -> String {
    let turns = if info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", info.turns) };
    let ago = human_ago(now_ms.saturating_sub(info.last_used_ms));
    style.dim(&format!("previous session: {turns}, {ago} · /resume"))
}

/// What `/resume` (or `-c`) says after loading a session.
pub fn resumed_line(info: &SessionInfo, now_ms: u64, style: Style) -> String {
    let turns = if info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", info.turns) };
    let last = info.current.as_ref().map_or(String::new(), |(p, m)| format!(", last on {p} · {m}"));
    let ago = human_ago(now_ms.saturating_sub(info.last_used_ms));
    format!("{} resumed {}", style.green("✓"), style.dim(&format!("{turns}{last}, {ago}")))
}

/// The marker file whose presence means the first-run hint was shown on this machine.
pub const FIRST_RUN_MARKER: &str = "first-run-done";

/// The first-run hint, once per machine: the text when there is something to say and it was
/// not shown before, and the marker is written so it is never shown again. A machine where
/// the marker cannot be written gets no hint rather than a hint on every start. `/doctor`
/// always has the full story.
pub fn first_run_hint_once(avail: &[Availability], state_dir: Option<&Path>) -> Option<String> {
    let dir = state_dir?;
    let marker = dir.join(FIRST_RUN_MARKER);
    if marker.exists() {
        return None;
    }
    let hint = doctor::startup_hint::first_run_hint(avail)?;
    std::fs::write(&marker, "shown\n").ok()?;
    Some(hint)
}
