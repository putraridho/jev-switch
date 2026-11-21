//! The turn log: one JSON row per attempt, appended to the file `JEV_CODE_LOG` names.

use serde_json::{json, Value};

use crate::backend::infrastructure::outcome_wire::turn_row;
use crate::turn::application::ports::{AttemptRow, TurnLog};
use crate::shared::infrastructure::jsonl_log as log;

/// Append the row of one attempt to the log at `path`.
fn append_attempt(path: &str, a: &AttemptRow<'_>) {
    let mut row = turn_row(a.provider, a.model, a.session, a.reason, a.jev, a.result, a.wall_ms);
    if let Value::Object(o) = &mut row {
        o.insert("attempt".into(), json!(a.attempt));
        o.insert("orchestrated".into(), json!(a.orchestrated));
        if a.cancelled {
            o.insert("error_kind".into(), json!("cancelled"));
            o.insert("cancelled".into(), json!(true));
        }
        if !a.notes.is_empty() {
            o.insert("failover_notes".into(), json!(a.notes));
        }
    }
    log::append(path, &row);
}

/// The JSON-lines turn log file as the [`TurnLog`].
pub struct FileTurnLog;

impl TurnLog for FileTurnLog {
    fn append(&self, path: &str, row: &AttemptRow<'_>) {
        append_attempt(path, row);
    }
}
