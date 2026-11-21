use super::upkeep::*;
use super::marks::{mark, Mark};
use super::*;

#[test]
fn disk_use_counts_by_project_and_clean_spares_pinned() {
    let (state, ..) = library("clean");
    let usage = disk_use(&scan_all(&FileSessions, &state));
    assert_eq!((usage.sessions, usage.by_project.len()), (3, 2));
    let old = scan_all(&FileSessions, &state).into_iter().find(|e| e.title() == "write docs").unwrap().path;
    mark(&FileSessions, &state, &old, &Mark::Pin(true)).unwrap();
    std::fs::write(state.join("sessions").join("garbage.json"), "{").unwrap();
    // 10 days: the 20-day-old session is pinned (kept), the garbage file goes.
    let (n, _) = clean(&FileSessions, &state, 10, NOW);
    assert_eq!(n, 1);
    assert_eq!(scan_all(&FileSessions, &state).len(), 3);
    mark(&FileSessions, &state, &old, &Mark::Pin(false)).unwrap();
    assert_eq!(clean(&FileSessions, &state, 10, NOW).0, 1);
    assert_eq!(scan_all(&FileSessions, &state).len(), 2);
}

#[test]
fn clean_with_enormous_age_keeps_readable_nonempty_sessions() {
    let (state, ..) = library("huge-clean");
    for days in [999_999_999_999, u64::MAX] {
        assert_eq!(clean(&FileSessions, &state, days, NOW), (0, 0));
        assert_eq!(scan_all(&FileSessions, &state).len(), 3);
        assert_eq!(clean(&FileSessions, &state, days, u64::MAX), (0, 0));
        assert_eq!(scan_all(&FileSessions, &state).len(), 3);
    }
}

#[test]
fn legacy_single_files_move_into_their_project_folder_once() {
    let state = temp("legacy");
    let project = state.join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let mut sessions = Sessions::new();
    sessions.record(turn(ProviderId::Claude, "old work", "ok", &[]));
    let saved = SavedSession { project: session_file::project_key(&project), saved_ms: NOW - DAY_MS, sessions, current: None };
    let legacy = session_file::legacy_session_file(&state, &project);
    write_atomic(&legacy, &saved.render()).unwrap();
    std::fs::write(state.join("sessions").join("zzz.json"), "not a session").unwrap();
    assert_eq!(migrate_legacy(&FileSessions, &SystemHost, &state), 1);
    assert!(!legacy.exists());
    let listed = session_file::list_sessions(&state, &project, NOW, None);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "old work");
    assert_eq!(migrate_legacy(&FileSessions, &SystemHost, &state), 0);
}
