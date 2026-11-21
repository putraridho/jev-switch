//! Files more than one worker touched.

use std::collections::BTreeMap;

use super::{WorkerInfo, WorkerState};
use crate::tui::render::Style;
use crate::tui::views::agent_render::{clean, plural};

/// Two or more workers touching one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub workers: Vec<String>,
}

/// The files more than one worker changed, in path order. Cancelled workers do not count.
pub fn file_conflicts(workers: &[WorkerInfo]) -> Vec<Conflict> {
    let mut by_file: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for w in workers.iter().filter(|w| w.state != WorkerState::Cancelled) {
        for f in &w.files {
            let ids = by_file.entry(f.as_str()).or_default();
            if !ids.contains(&w.id) {
                ids.push(w.id.clone());
            }
        }
    }
    by_file.into_iter().filter(|(_, ids)| ids.len() > 1).map(|(p, ids)| Conflict { path: p.into(), workers: ids }).collect()
}

/// A warning block for [`file_conflicts`]; `None` when the workers do not overlap.
pub fn conflict_warning(conflicts: &[Conflict], style: Style) -> Option<String> {
    if conflicts.is_empty() {
        return None;
    }
    let head = format!("! {} touched by more than one worker; merging the second will conflict:", plural(conflicts.len(), "file", "files"));
    let mut out = vec![format!("  {}", style.yellow(&head))];
    out.extend(conflicts.iter().map(|c| format!("    {} {}", clean(&c.path), style.dim(&format!("({})", c.workers.join(", "))))));
    Some(out.join("\n"))
}
