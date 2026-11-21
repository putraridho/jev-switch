use super::*;
use crate::tui::render::display_width;
use crate::tui::views::agent_orch::flow::*;
use crate::tui::views::agent_workers::{WorkerInfo, WorkerState};

fn workers() -> Vec<WorkerInfo> {
    vec![WorkerInfo::new("w1", "codex", "gpt-x", "parser", WorkerState::Done), WorkerInfo::new("w2", "gemini", "flash", "lexer", WorkerState::Running)]
}

#[test]
fn flow_diagram_has_planner_lanes_and_merge() {
    let d = flow_diagram("claude opus", &workers(), 90, OFF);
    let rows: Vec<&str> = d.lines().collect();
    assert!(rows[0].contains("planner · claude opus"));
    assert!(rows[1].contains("├──▶") && rows[1].contains("w1") && rows[1].contains("done"));
    assert!(rows[2].contains("w2") && rows[2].contains("running"));
    assert!(d.contains("1 of 2 merged back") && d.contains("└ merge → review → report"));
    for l in flow_diagram("claude opus", &workers(), 30, OFF).lines() {
        assert!(display_width(l) <= 30, "{l:?}");
    }
    assert!(flow_diagram("p", &[], 60, OFF).contains("no workers yet"));
}

#[test]
fn markdown_export_fences_the_diagram() {
    let m = flow_markdown("claude opus", &workers());
    assert!(m.starts_with("## Orchestration\n\n```text\n") && m.ends_with("```\n") && !m.contains('\x1b'));
}
