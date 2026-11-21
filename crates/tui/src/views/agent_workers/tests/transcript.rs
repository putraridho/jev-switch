use super::*;
use crate::views::agent_workers::transcript::*;

fn act(id: &str, steps: &[&str], children: Vec<Activity>) -> Activity {
    Activity { worker: w(id, WorkerState::Done), steps: steps.iter().map(|s| s.to_string()).collect(), children }
}

#[test]
fn nested_transcript_collapses_to_one_line_per_worker_with_a_tree_gutter() {
    let g = [act("w1", &["Read a.rs", "Edit a.rs"], vec![]), act("w2", &["Bash cargo test"], vec![])];
    let t = nested_transcript("split the work", &g, false, 80, OFF);
    let rows: Vec<&str> = t.lines().collect();
    assert_eq!(rows.len(), 3);
    assert!(rows[1].contains("├─") && rows[1].contains("2 steps"));
    assert!(rows[2].contains("└─") && rows[2].contains("1 step "));
}

#[test]
fn expanded_transcript_streams_tool_calls_and_nests_depth() {
    let g = [act("w1", &["Read a.rs"], vec![act("w1a", &["Grep x"], vec![])]), act("w2", &["Edit b.rs"], vec![])];
    let t = nested_transcript("t", &g, true, 80, OFF);
    let rows: Vec<&str> = t.lines().collect();
    assert!(rows.iter().any(|r| r.contains("│  Read a.rs")), "{t}");
    assert!(rows.iter().any(|r| r.contains("│  └─") && r.contains("w1a")), "{t}");
    assert!(rows.iter().any(|r| r.contains("   Edit b.rs") && !r.contains('│')), "last branch drops the trunk: {t}");
}
