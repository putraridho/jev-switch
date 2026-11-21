//! Tool-call arguments: parsing what the planner sent (reading only the real keys, so a
//! `model` never passes through) and running one tool against the delegation API.

use std::time::Duration;

use serde_json::{json, Value};

use super::json_rpc::Server;
use crate::domain::delegation::DelegateRequest;

/// The request the planner's `delegate` arguments mean. Reads `brief`, `files`,
/// `read_only` and `review` and NOTHING else: a `model` (or any other) key is dropped here,
/// so no model choice can pass through the tool.
pub(super) fn parse_delegate(args: &Value) -> Result<DelegateRequest, String> {
    let brief = args["brief"].as_str().map(str::trim).filter(|b| !b.is_empty()).ok_or("`brief` is required: a non-empty string")?;
    let files = string_list(args, "files")?.unwrap_or_default();
    let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
    Ok(DelegateRequest { brief: brief.to_string(), files, read_only: flag("read_only"), review: flag("review") })
}

/// A `files` argument: absent (or null) is `None`, otherwise a list of strings.
fn string_list(args: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items.iter().map(|f| f.as_str().map(String::from).ok_or(format!("`{key}` must be a list of strings"))).collect::<Result<_, _>>().map(Some),
        Some(_) => Err(format!("`{key}` must be a list of strings")),
    }
}

/// `wait`'s optional `task_ids` and `timeout_seconds`.
fn parse_wait(args: &Value) -> Result<(Option<Vec<String>>, Option<Duration>), String> {
    let ids = string_list(args, "task_ids")?;
    let timeout = match args.get("timeout_seconds") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Duration::from_secs(v.as_u64().ok_or("`timeout_seconds` must be a non-negative integer")?)),
    };
    Ok((ids, timeout))
}

/// What the planner reads after starting a task.
fn started_text(task_id: &str) -> String {
    format!(
        "started task {task_id}. The worker runs in the background: carry on with other work, then call wait (no arguments \
         collects every task) to get its result. Nothing lands in your working tree until you call wait."
    )
}

/// `show_diff`'s optional `file` and `offset`.
fn parse_show_diff(args: &Value) -> Result<(Option<String>, usize), String> {
    let file = match args.get("file") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if !f.is_empty() => Some(f.clone()),
        Some(_) => return Err("`file` must be a non-empty string".into()),
    };
    let offset = match args.get("offset") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().ok_or("`offset` must be a non-negative integer")? as usize,
    };
    Ok((file, offset))
}

fn worker_id_arg(args: &Value) -> Result<String, String> {
    args["worker_id"].as_str().filter(|w| !w.is_empty()).map(String::from).ok_or_else(|| "`worker_id` is required".to_string())
}

fn text_result(text: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": is_error })
}

/// Run one tool. `Err` is a malformed call (a protocol error); a tool that ran and
/// failed is an `Ok` result flagged `isError`, which is how MCP lets the model see it.
pub(super) async fn call_tool(server: &Server, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("tools/call needs a tool `name`")?;
    let args = &params["arguments"];
    match name {
        "delegate" => Ok(outcome(server.api.start(parse_delegate(args)?).await.map(|id| started_text(&id)))),
        "wait" => {
            let (ids, timeout) = parse_wait(args)?;
            Ok(outcome(server.api.wait(ids, timeout).await))
        }
        "show_diff" => {
            let (file, offset) = parse_show_diff(args)?;
            Ok(outcome(server.api.show_diff(worker_id_arg(args)?, file, offset).await))
        }
        "merge" => {
            let files = string_list(args, "files")?;
            Ok(outcome(server.api.merge(worker_id_arg(args)?, files).await))
        }
        "discard" => Ok(outcome(server.api.discard(worker_id_arg(args)?).await)),
        "undo" => Ok(outcome(server.api.undo(worker_id_arg(args)?).await)),
        other => Err(format!("unknown tool `{other}` (delegate, wait, show_diff, merge, discard, undo)")),
    }
}

fn outcome(r: Result<String, String>) -> Value {
    match r {
        Ok(text) => text_result(text, false),
        Err(text) => text_result(text, true),
    }
}
