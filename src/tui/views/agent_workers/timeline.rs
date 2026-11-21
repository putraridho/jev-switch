//! The Gantt timeline of the workers.

use crate::tui::render::{display_width, human_duration, Style};
use crate::tui::views::agent_render::clamp_width;

/// One worker's time on the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// The stretch inside the span spent waiting for an approval.
    pub waited: Option<(u64, u64)>,
}

/// Gantt lanes: `█` working, `░` waiting for approval, blank idle; the worker that ends
/// last is the critical path.
pub fn timeline(spans: &[Span], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let Some(total) = spans.iter().map(|s| s.end_ms).max().filter(|t| *t > 0) else {
        return format!("  {}", style.dim("No timeline yet: no worker has run."));
    };
    let label_w = spans.iter().map(|s| display_width(&s.id)).max().unwrap_or(0);
    let cells = width.saturating_sub(label_w + 24).clamp(8, 60);
    let cell_of = |ms: u64| ((ms as u128 * cells as u128) / total as u128) as usize;
    let mut out = vec![format!("  {}  {}", style.bold("Timeline"), style.dim(&format!("0 … {} (█ working  ░ waiting)", human_duration(total))))];
    for s in spans {
        let from = cell_of(s.start_ms).min(cells - 1);
        let to = cell_of(s.end_ms).clamp(from + 1, cells);
        let lane: String = (0..cells)
            .map(|c| {
                let waiting = s.waited.is_some_and(|(ws, we)| c >= cell_of(ws) && c < cell_of(we).max(cell_of(ws) + 1));
                match (c >= from && c < to, waiting) {
                    (false, _) => ' ',
                    (true, true) => '░',
                    (true, false) => '█',
                }
            })
            .collect();
        let tag = if s.end_ms == total { "  ◀ critical path" } else { "" };
        out.push(format!("  {:<label_w$} │{lane}│ {}{}", s.id, human_duration(s.end_ms.saturating_sub(s.start_ms)), style.yellow(tag)));
    }
    out.join("\n")
}
