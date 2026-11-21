//! Sample saved sessions on disk for tests of the session library and of what renders it
//! (the tui's search hits): two projects, three sessions of known age.

use std::path::{Path, PathBuf};

use jev_backend::ProviderId;
use crate::application::session_library::DAY_MS;
use crate::domain::saved_session::SavedSession;
use crate::domain::session::Sessions;
use crate::domain::turn_record::TurnRecord;
use crate::infrastructure::session_file;
use jev_shared::infrastructure::private_file::write_atomic;

/// "Now" for every fixture: day 100.
pub const NOW: u64 = 100 * DAY_MS;

/// A fresh, empty folder for one test.
pub fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jev-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One finished turn.
pub fn turn(p: ProviderId, prompt: &str, reply: &str, tools: &[&str]) -> TurnRecord {
    TurnRecord { prompt: prompt.into(), provider: p, model: "m".into(), reply: reply.into(), tools: tools.iter().map(|s| s.to_string()).collect(), ok: true }
}

/// Save a session of `turns` for `project`, `at_days_ago` days before [`NOW`].
pub fn put(state: &Path, project: &Path, at_days_ago: u64, turns: Vec<TurnRecord>) -> PathBuf {
    let mut sessions = Sessions::new();
    for t in turns {
        sessions.record(t);
    }
    let saved = SavedSession { project: session_file::project_key(project), saved_ms: NOW - at_days_ago * DAY_MS, sessions, current: None };
    let path = session_file::new_session_file(state, project, saved.saved_ms);
    write_atomic(&path, &super::saved_session_wire::render(&saved)).unwrap();
    path
}

/// A state folder with two projects and three sessions: (state, project a, project b).
pub fn library(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let state = temp(name);
    let (a, b) = (state.join("proj-a"), state.join("proj-b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    put(&state, &a, 1, vec![turn(ProviderId::Claude, "fix the lexer", "Done, lexer fixed.", &["Edit src/lexer.rs"])]);
    put(&state, &a, 20, vec![turn(ProviderId::Codex, "write docs", "Docs written about the parser.", &[])]);
    put(&state, &b, 3, vec![turn(ProviderId::Codex, "parser bug", "Found it in tokens.", &["Edit src/parser.rs"])]);
    (state, a, b)
}
