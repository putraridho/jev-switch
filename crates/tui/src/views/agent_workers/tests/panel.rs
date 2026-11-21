use super::*;
use crate::render::{display_width, strip_ansi};
use crate::views::agent_workers::panel::*;

#[test]
fn panel_sorts_what_needs_you_first_and_names_each_state_in_words() {
    let ws = [w("w1", WorkerState::Done), w("w2", WorkerState::WaitingApproval), w("w3", WorkerState::Running)];
    let p = agents_panel(&ws, 100, OFF);
    let rows: Vec<&str> = p.lines().collect();
    assert!(rows[0].contains("Agents") && rows[0].contains("1 of 3"), "{p}");
    assert!(rows[1].contains("waiting") && rows[1].contains("w2"));
    assert!(rows[2].contains("running"));
    assert!(rows[3].contains("done") && rows[3].contains("1.2K tok") && rows[3].contains("$0.04"));
    assert!(!p.contains('\x1b'));
}

#[test]
fn a_running_worker_shows_its_last_action_instead_of_the_task() {
    let mut r = w("w1", WorkerState::Running);
    r.last_action = Some("editing src/parser.rs".into());
    assert!(agents_panel(&[r], 100, OFF).contains("editing src/parser.rs"));
}

#[test]
fn narrow_terminals_get_one_short_line_per_worker() {
    let p = agents_panel(&[w("w1", WorkerState::Running), w("w2", WorkerState::Done)], 40, OFF);
    for l in p.lines() {
        assert!(display_width(l) <= 40, "{l:?}");
    }
    assert_eq!(p.lines().count(), 3);
}

#[test]
fn long_text_never_overflows_the_width() {
    let mut x = w("w1", WorkerState::Running);
    x.task = "a very long task description that keeps going and going and going".into();
    for width in [40, 69, 70, 80, 120] {
        for l in agents_panel(&[x.clone()], width, OFF).lines() {
            assert!(display_width(l) <= width, "{width}: {l:?}");
        }
    }
}

#[test]
fn styling_only_adds_escapes() {
    let ws = [w("w1", WorkerState::Failed), w("w2", WorkerState::Done)];
    assert_eq!(strip_ansi(&agents_panel(&ws, 100, ON)), agents_panel(&ws, 100, OFF));
    assert!(agents_panel(&ws, 100, ON).contains('\x1b'));
}

#[test]
fn empty_panel_says_how_to_get_workers() {
    assert!(agents_panel(&[], 80, OFF).contains("/orchestrate on"));
}

#[test]
fn footer_counts_workers_and_what_is_running() {
    assert_eq!(workers_footer(&[]), None);
    let ws = [w("a", WorkerState::Running), w("b", WorkerState::Running), w("c", WorkerState::Done), w("d", WorkerState::Queued)];
    assert_eq!(workers_footer(&ws).as_deref(), Some("4 workers · 2 running"));
    assert_eq!(workers_footer(&ws[..1]).as_deref(), Some("1 worker · 1 running"));
}
