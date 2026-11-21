//! What is happening now: the working row and the planner's workers above the box.

use super::theme::Theme;
use crate::app::screen_state::App;
use crate::render::{truncate_to_width, Spinner, Style};

/// What the model is doing, said as an action: `Bash cargo test` → `Running cargo test`.
pub fn doing(activity: Option<&str>) -> String {
    let (verb, rest) = doing_parts(activity);
    if rest.is_empty() { verb } else { format!("{verb} {rest}") }
}

/// [`doing`] in its two parts, the verb and what it acts on, so they can be weighted apart.
fn doing_parts(activity: Option<&str>) -> (String, String) {
    let Some(raw) = activity.map(str::trim).filter(|a| !a.is_empty()) else { return ("Thinking".to_string(), String::new()) };
    let a = raw.strip_prefix("⎿ ").unwrap_or(raw);
    let (tool, rest) = a.split_once(' ').unwrap_or((a, ""));
    let verb = match tool.to_ascii_lowercase().as_str() {
        "bash" | "shell" => "Running",
        "read" => "Reading",
        "edit" | "multiedit" => "Editing",
        "write" => "Writing",
        "grep" | "glob" | "ls" => "Searching",
        "webfetch" | "websearch" | "web_search" => "Looking up",
        "task" | "agent" | "delegate" | "spawn_agent" | "invoke_subagent" | "subagent" => "Delegating",
        t if t.ends_with("__delegate") => "Delegating",
        _ => return (a.to_string(), String::new()),
    };
    (verb.to_string(), rest.to_string())
}

/// The working row: `⠹ Running cargo test 12s`, the verb dim and what it acts on in the accent,
/// then how long. Without colour the spinner is a still mark (no colour means no animation).
pub fn working_line(work: &crate::app::screen_state::Working, elapsed: std::time::Duration, width: usize, style: Style) -> String {
    let spinner = if style.enabled { Spinner::frame_for(elapsed) } else { "•" };
    let secs = elapsed.as_secs();
    let time = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    let (verb, rest) = doing_parts(work.activity.as_deref());
    let what = if rest.is_empty() { style.bold(&verb) } else { format!("{} {}", style.dim(&verb), style.accent(&rest)) };
    truncate_to_width(&format!("{} {what} {}", style.accent(spinner), style.dim(&time)), width)
}

/// The most workers listed by name above the box; the rest are counted.
const WORKER_ROWS: usize = 4;

/// What the planner's workers are doing, above the box: a heading row,
/// `▾ Workers · 1 running · 1 to review`, then a row each (`▶ w1  gemini · argon  running 0:42`,
/// `✓ w2  codex  ready to review`). Nothing when no worker is open.
pub(super) fn workers_rows(app: &App, w: usize, style: Style) -> Vec<String> {
    if app.workers.is_empty() {
        return Vec::new();
    }
    let th = Theme(style);
    let names = Theme::provider_theme(style);
    let running = app.workers.iter().filter(|x| x.running).count();
    let waiting = app.workers.len() - running;
    let mut title = String::from("Workers");
    if running > 0 {
        title.push_str(&format!(" · {running} running"));
    }
    if waiting > 0 {
        title.push_str(&format!(" · {waiting} to review"));
    }
    let mut rows = vec![truncate_to_width(&format!("  {} {}", th.muted("▾"), th.muted(&th.strong(&title))), w)];
    for wk in app.workers.iter().take(WORKER_ROWS) {
        let (glyph, state) = if wk.running {
            (style.accent("▶"), style.dim(&format!("running {}", clock(wk.age_ms))))
        } else {
            (style.yellow("✓"), style.yellow("ready to review"))
        };
        let who = if wk.model.is_empty() { names.name(&wk.provider) } else { format!("{} {} {}", names.name(&wk.provider), style.dim("·"), wk.model) };
        let calls = if wk.running && wk.tools > 0 { style.dim(&format!(" · {} tool calls", wk.tools)) } else { String::new() };
        rows.push(truncate_to_width(&format!("  {glyph} {}  {who}   {state}{calls}", style.bold(&wk.id)), w));
        // What a running worker is doing right now, under it.
        if let Some(doing) = wk.activity.as_deref().filter(|_| wk.running) {
            rows.push(truncate_to_width(&format!("      {}", th.muted(&format!("⎿ {doing}"))), w));
        }
    }
    if app.workers.len() > WORKER_ROWS {
        rows.push(truncate_to_width(&format!("    {}", style.dim(&format!("… and {} more", app.workers.len() - WORKER_ROWS))), w));
    }
    rows
}

/// `0:42`, `12:05`: a worker's age.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
