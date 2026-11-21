//! Shared fixtures for the session library tests.

use std::path::{Path, PathBuf};

use super::search::Hit;
use super::*;
use crate::backend::ProviderId;
use crate::session::domain::saved_session::SavedSession;
use crate::session::domain::session::Sessions;
use crate::session::domain::turn_record::TurnRecord;
use crate::session::infrastructure::session_file;
use crate::shared::infrastructure::private_file::write_atomic;

mod searching;
mod export_import;
mod disk_and_clean;

pub(super) const NOW: u64 = 100 * DAY_MS;

pub(super) fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jev-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub(super) fn turn(p: ProviderId, prompt: &str, reply: &str, tools: &[&str]) -> TurnRecord {
    TurnRecord { prompt: prompt.into(), provider: p, model: "m".into(), reply: reply.into(), tools: tools.iter().map(|s| s.to_string()).collect(), ok: true }
}

pub(super) fn put(state: &Path, project: &Path, at_days_ago: u64, turns: Vec<TurnRecord>) -> PathBuf {
    let mut sessions = Sessions::new();
    for t in turns {
        sessions.record(t);
    }
    let saved = SavedSession { project: session_file::project_key(project), saved_ms: NOW - at_days_ago * DAY_MS, sessions, current: None };
    let path = session_file::new_session_file(state, project, saved.saved_ms);
    write_atomic(&path, &saved.render()).unwrap();
    path
}

pub(super) fn titles(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.entry.title()).collect()
}

pub(super) fn library(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let state = temp(name);
    let (a, b) = (state.join("proj-a"), state.join("proj-b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    put(&state, &a, 1, vec![turn(ProviderId::Claude, "fix the lexer", "Done, lexer fixed.", &["Edit src/lexer.rs"])]);
    put(&state, &a, 20, vec![turn(ProviderId::Codex, "write docs", "Docs written about the parser.", &[])]);
    put(&state, &b, 3, vec![turn(ProviderId::Codex, "parser bug", "Found it in tokens.", &["Edit src/parser.rs"])]);
    (state, a, b)
}
