//! The card shown when a worker finishes.

use super::{WorkerInfo, WorkerState};
use crate::render::{human_count, human_duration, Style, Theme};
use crate::views::agent_render::{clamp_width, fit, usd};

/// What a finished worker leaves for the result card.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultFacts {
    pub worker: WorkerInfo,
    pub summary: String,
    pub tests: Option<String>,
    /// Why it failed, when it did.
    pub error: Option<String>,
}

/// The card shown when a worker finishes: outcome, files, tests, cost and the actions.
pub fn result_card(r: &ResultFacts, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let w = &r.worker;
    let theme = Theme::new(style);
    let mut out = vec![format!(
        "  {} {}  {}  {}",
        w.state.paint(style, &format!("{} {}", w.state.glyph(), w.state.word())),
        w.id,
        theme.provider(&w.provider, &format!("{} {}", w.provider, w.model)),
        style.dim(&human_duration(w.elapsed_ms))
    )];
    let pad = "    ";
    let room = width.saturating_sub(pad.len());
    if !r.summary.is_empty() {
        out.push(format!("{pad}{}", fit(&r.summary, room)));
    }
    if let Some(e) = &r.error {
        out.push(format!("{pad}{}", style.red(&fit(&format!("reason: {e}"), room))));
    }
    match w.files.len() {
        0 => out.push(format!("{pad}{}", style.dim("files: none"))),
        n => out.push(format!("{pad}files ({n}): {}", fit(&w.files.join(", "), room.saturating_sub(12)))),
    }
    if let Some(t) = &r.tests {
        out.push(format!("{pad}tests: {}", fit(t, room.saturating_sub(7))));
    }
    let spend = [(w.tokens > 0).then(|| format!("{} tok", human_count(w.tokens))), w.cost_usd.map(usd)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    if !spend.is_empty() {
        out.push(format!("{pad}cost: {spend}"));
    }
    let actions = if w.state == WorkerState::Done {
        "[r] review  [a] accept  [d] discard".to_string()
    } else {
        format!("[t] retry  [d] discard   next: read the reason, then retry {} or do the piece yourself", w.id)
    };
    out.push(format!("{pad}{}", style.bold(&fit(&actions, room))));
    out.join("\n")
}
