//! The nested transcript of a delegate: a worker tree with its steps.

use super::WorkerInfo;
use crate::render::{display_width, human_duration, truncate_to_width, Style};
use crate::views::agent_render::{clamp_width, fit, plural};

/// A worker's recorded steps, for the nested transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub worker: WorkerInfo,
    /// One line per tool call, oldest first.
    pub steps: Vec<String>,
    /// The workers this one spawned.
    pub children: Vec<Activity>,
}

/// The `⇉ Delegate` row with its workers' activity under a tree gutter: one summary line
/// per worker when collapsed, the full tool-call stream when `expanded`.
pub fn nested_transcript(task: &str, group: &[Activity], expanded: bool, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  ⇉ Delegate {}", fit(task, width.saturating_sub(14)))];
    for (i, a) in group.iter().enumerate() {
        tree_node(a, "    ", i + 1 == group.len(), expanded, width, style, &mut out);
    }
    out.join("\n")
}

fn tree_node(a: &Activity, prefix: &str, last: bool, expanded: bool, width: usize, s: Style, out: &mut Vec<String>) {
    let (branch, trunk) = if last { ("└─", "   ") } else { ("├─", "│  ") };
    let w = &a.worker;
    let summary = format!("{} {} {} {} · {} · {}", w.state.glyph(), w.id, w.provider, w.state.word(), plural(a.steps.len(), "step", "steps"), human_duration(w.elapsed_ms));
    out.push(format!("{prefix}{}{}", s.dim(branch), w.state.paint(s, &truncate_to_width(&summary, width.saturating_sub(display_width(prefix) + 2)))));
    let inner = format!("{prefix}{trunk}");
    if expanded {
        for step in &a.steps {
            out.push(format!("{inner}{}", s.dim(&fit(step, width.saturating_sub(display_width(&inner))))));
        }
    }
    for (i, c) in a.children.iter().enumerate() {
        tree_node(c, &inner, i + 1 == a.children.len(), expanded, width, s, out);
    }
}
