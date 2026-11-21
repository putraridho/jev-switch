//! The orchestration log: one row per worker attempt, and the reads `/workflows` makes of it.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use jev_backend::infrastructure::outcome_wire::turn_row;
use crate::application::ports::{AttemptLog, DelegationLog};
use jev_shared::infrastructure::jsonl_log as log;

/// The project folder as rows and records name it: canonical when it can be.
pub fn handoff_key(dir: &Path) -> String {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().into_owned()
}

/// One worker attempt from the orchestration log.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Delegation {
    pub ts_ms: u64,
    pub provider: String,
    pub model: String,
    pub ok: bool,
    #[serde(default)]
    pub error_kind: Option<String>,
    pub wall_ms: u64,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

/// The project's delegations from the log at `log`, newest last, at most `limit`. Rows
/// written before the log named the project are left out, as is anything unreadable.
pub fn recent_delegations(log: &Path, repo: &Path, limit: usize) -> Vec<Delegation> {
    let key = handoff_key(repo);
    let Ok(text) = std::fs::read_to_string(log) else { return Vec::new() };
    let mut rows: Vec<Delegation> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Delegation>(l).ok())
        .filter(|d| d.kind.as_deref() == Some("delegate") && d.repo.as_deref() == Some(key.as_str()))
        .collect();
    let skip = rows.len().saturating_sub(limit);
    rows.drain(..skip);
    rows
}

/// Append the row for one worker attempt to the log at `path`; `repo` is the repository the
/// worker was cut from.
fn append_attempt(path: &str, planner: &str, repo: &Path, a: &AttemptLog) {
    let reason = format!("delegated {} brief", a.task);
    let mut row = turn_row(a.spec.provider, a.spec.id, None, &reason, a.jev, a.result, a.wall_ms);
    if let Value::Object(o) = &mut row {
        o.insert("kind".into(), json!("delegate"));
        o.insert("planner".into(), json!(planner));
        o.insert("worker_id".into(), json!(a.id));
        o.insert("task".into(), json!(a.task.as_str()));
        o.insert("attempt".into(), json!(a.attempt));
        // When the worker ran, so parallel workers' overlap can be read off the log. The finish
        // is the start plus the measured wall time, so it can never precede the start even if
        // the wall clock steps back mid-run.
        o.insert("started_at".into(), json!(a.started_ms));
        o.insert("finished_at".into(), json!(a.started_ms.saturating_add(a.wall_ms)));
        o.insert("duration_ms".into(), json!(a.wall_ms));
        o.insert("read_only".into(), json!(a.read_only));
        o.insert("files_changed".into(), json!(a.files));
        o.insert("landing".into(), json!(a.landing));
        o.insert("in_place".into(), json!(a.in_place));
        o.insert("tool_calls".into(), json!(a.tools));
        o.insert("repo".into(), json!(handoff_key(repo)));
    }
    log::append(path, &row);
}

/// The JSON-lines log file as the [`DelegationLog`].
pub struct JsonlLog;

impl DelegationLog for JsonlLog {
    fn append(&self, path: &str, planner: &str, repo: &Path, a: &AttemptLog) {
        append_attempt(path, planner, repo, a);
    }
}
