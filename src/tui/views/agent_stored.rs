//! `/agents` and `/orchestration` as the REPL runs them: the worker records on disk made
//! into the views' facts. (The records are what a planner can still merge or discard;
//! a live per-tool feed from running workers is not available to the REPL yet.)

use crate::tui::views::agent_orch::flow::flow_diagram;
use crate::tui::views::agent_workers::conflicts::{conflict_warning, file_conflicts};
use crate::tui::views::agent_workers::panel::agents_panel;
use crate::tui::views::agent_workers::{WorkerInfo, WorkerState};
use crate::orchestration::infrastructure::worker_file::StoredWorker;
use crate::tui::render::Style;

/// The panel facts of recorded workers: a running one is running, any other is done and
/// waiting for its merge or discard. `now_ms` dates the elapsed time.
pub fn infos(stored: &[StoredWorker], now_ms: u64) -> Vec<WorkerInfo> {
    stored
        .iter()
        .map(|s| {
            let (state, task) = if s.running { (WorkerState::Running, "working") } else { (WorkerState::Done, "finished, waiting for the planner to merge or discard") };
            let mut w = WorkerInfo::new(&s.id, &s.provider, &s.model, task, state);
            w.elapsed_ms = now_ms.saturating_sub(s.created_ms);
            w.files = s.merged.clone();
            w
        })
        .collect()
}

/// `/agents`.
pub fn agents_text(stored: &[StoredWorker], now_ms: u64, width: usize, style: Style) -> String {
    let workers = infos(stored, now_ms);
    let mut out = agents_panel(&workers, width, style);
    if let Some(w) = conflict_warning(&file_conflicts(&workers), style) {
        out.push_str(&format!("\n{w}"));
    }
    out
}

/// `/orchestration`: whether it is on, and the turn drawn as a flow.
pub fn orchestration_text(on: bool, stored: &[StoredWorker], now_ms: u64, width: usize, style: Style) -> String {
    let state = if on { style.green("on") } else { style.dim("off") };
    let diagram = flow_diagram("chosen per turn (Opus or Fable class)", &infos(stored, now_ms), width, style);
    format!("  {}  {state}\n{diagram}", style.bold("Orchestration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, running: bool) -> StoredWorker {
        StoredWorker { id: id.into(), provider: "codex".into(), model: "gpt-x".into(), running, created_ms: 1_000, merged: vec![] }
    }

    #[test]
    fn records_become_running_or_done_workers_with_elapsed_time() {
        let w = infos(&[rec("a", true), rec("b", false)], 61_000);
        assert_eq!(w[0].state, WorkerState::Running);
        assert_eq!(w[1].state, WorkerState::Done);
        assert_eq!(w[0].elapsed_ms, 60_000);
    }

    #[test]
    fn agents_text_lists_records_or_explains_the_empty_case() {
        let t = agents_text(&[rec("a", true), rec("b", false)], 5_000, 100, Style::off());
        assert!(t.contains("running") && t.contains("codex gpt-x"), "{t}");
        assert!(agents_text(&[], 0, 100, Style::off()).contains("No workers yet"));
    }

    #[test]
    fn orchestration_text_shows_state_and_diagram() {
        let t = orchestration_text(true, &[rec("a", false)], 5_000, 100, Style::off());
        assert!(t.contains("Orchestration  on") && t.contains("planner ·") && t.contains("merge → review → report"), "{t}");
        assert!(orchestration_text(false, &[], 0, 80, Style::off()).contains("off"));
    }
}
