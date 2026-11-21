//! The flow diagram of a whole turn and its Markdown export.

use crate::tui::render::{display_width, Style, Theme};
use crate::tui::views::agent_render::{clamp_width, fit};
use crate::tui::views::agent_workers::{WorkerInfo, WorkerState};

/// The whole turn as a diagram: planner at the top, one lane per worker, merge at the
/// bottom.
pub fn flow_diagram(planner: &str, workers: &[WorkerInfo], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  ┌ {}", style.bold(&format!("planner · {}", fit(planner, width.saturating_sub(16)))))];
    if workers.is_empty() {
        out.push(format!("  │ {}", style.dim("(no workers yet)")));
    }
    for w in workers {
        let head = format!("  ├──▶ {} {} {} ", w.state.glyph(), w.id, w.state.word());
        let who = fit(&format!("{} {}", w.provider, w.model), width.saturating_sub(display_width(&head)));
        let room = width.saturating_sub(display_width(&head) + display_width(&who) + 1);
        let task = if room > 3 { format!(" {}", style.dim(&fit(&w.task, room))) } else { String::new() };
        out.push(format!("{head}{}{task}", Theme::new(style).provider(&w.provider, &who)));
    }
    let done = workers.iter().filter(|w| w.state == WorkerState::Done).count();
    out.push(format!("  │ {}", style.dim(&format!("{done} of {} merged back", workers.len()))));
    out.push(format!("  └ {}", style.bold("merge → review → report")));
    out.join("\n")
}

/// The diagram as a Markdown section, for export.
pub fn flow_markdown(planner: &str, workers: &[WorkerInfo]) -> String {
    format!("## Orchestration\n\n```text\n{}\n```\n", flow_diagram(planner, workers, 100, Style::off()))
}
