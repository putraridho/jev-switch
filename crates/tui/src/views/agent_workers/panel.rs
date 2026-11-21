//! The `/agents` panel and the footer cue.

use super::{WorkerInfo, WorkerState};
use crate::render::{display_width, human_count, human_duration, Style, Theme};
use crate::views::agent_render::{bar, clamp_width, fit, plural, usd};

/// Below this width the panel is one short line per worker.
const NARROW: usize = 70;

fn sorted(workers: &[WorkerInfo]) -> Vec<&WorkerInfo> {
    let mut v: Vec<&WorkerInfo> = workers.iter().collect();
    v.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.id.cmp(&b.id)));
    v
}

/// The `/agents` panel: every worker, needing-you first.
pub fn agents_panel(workers: &[WorkerInfo], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    if workers.is_empty() {
        return format!("  {}", style.dim("No workers yet. Workers appear here when a planner hands pieces off (/orchestrate on)."));
    }
    let theme = Theme::new(style);
    let done = workers.iter().filter(|w| w.state == WorkerState::Done).count();
    let mut out = vec![format!("  {}  {}", style.bold("Agents"), style.dim(&format!("{} {done} of {}", bar(done, workers.len(), 8), workers.len())))];
    for w in sorted(workers) {
        out.push(if width < NARROW { narrow_row(w, width, style, &theme) } else { wide_row(w, width, style, &theme) });
    }
    out.join("\n")
}

fn narrow_row(w: &WorkerInfo, width: usize, s: Style, theme: &Theme) -> String {
    let lead = format!("  {} {} {} ", w.state.glyph(), w.id, w.provider);
    let room = width.saturating_sub(display_width(&lead));
    format!("  {} {} {} {}", w.state.paint(s, w.state.glyph()), w.id, theme.provider(&w.provider, &w.provider), s.dim(&fit(&w.task, room)))
}

fn wide_row(w: &WorkerInfo, width: usize, s: Style, theme: &Theme) -> String {
    let indent = "  ".repeat(w.depth + 1);
    let state = format!("{} {:<9}", w.state.glyph(), w.state.word());
    let mut parts = vec![human_duration(w.elapsed_ms)];
    if w.tokens > 0 {
        parts.push(format!("{} tok", human_count(w.tokens)));
    }
    if let Some(c) = w.cost_usd {
        parts.push(usd(c));
    }
    if !w.files.is_empty() {
        parts.push(plural(w.files.len(), "file", "files"));
    }
    let meta = parts.join(" · ");
    let who = format!("{} {}", w.provider, w.model);
    let fixed = display_width(&indent) + display_width(&state) + 1 + display_width(&w.id) + 2 + display_width(&who) + 2 + display_width(&meta) + 2;
    let detail = w.last_action.as_deref().filter(|_| w.state.live()).unwrap_or(&w.task);
    let detail = fit(detail, width.saturating_sub(fixed));
    format!("{indent}{} {}  {}  {}  {}", w.state.paint(s, &state), w.id, theme.provider(&w.provider, &who), s.dim(&meta), detail)
}

/// The quiet footer cue: `4 workers · 2 running`; nothing when no worker exists.
pub fn workers_footer(workers: &[WorkerInfo]) -> Option<String> {
    if workers.is_empty() {
        return None;
    }
    let count = |st: WorkerState| workers.iter().filter(|w| w.state == st).count();
    let mut s = plural(workers.len(), "worker", "workers");
    for (st, label) in [(WorkerState::Running, "running"), (WorkerState::WaitingApproval, "waiting"), (WorkerState::Failed, "failed")] {
        if count(st) > 0 {
            s.push_str(&format!(" · {} {label}", count(st)));
        }
    }
    Some(s)
}
