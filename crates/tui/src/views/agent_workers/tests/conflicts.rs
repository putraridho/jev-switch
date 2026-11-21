use super::*;
use crate::views::agent_workers::conflicts::*;

#[test]
fn conflicts_name_files_two_workers_touch_and_ignore_cancelled_ones() {
    let mut a = w("w1", WorkerState::Done);
    a.files = vec!["src/a.rs".into(), "src/b.rs".into()];
    let mut b = w("w2", WorkerState::Running);
    b.files = vec!["src/b.rs".into()];
    let mut c = w("w3", WorkerState::Cancelled);
    c.files = vec!["src/a.rs".into()];
    let cs = file_conflicts(&[a.clone(), b, c]);
    assert_eq!(cs, vec![Conflict { path: "src/b.rs".into(), workers: vec!["w1".into(), "w2".into()] }]);
    let warn = conflict_warning(&cs, OFF).unwrap_or_default();
    assert!(warn.contains("src/b.rs") && warn.contains("w1, w2") && warn.contains("will conflict"));
    assert_eq!(conflict_warning(&file_conflicts(&[a]), OFF), None);
}
