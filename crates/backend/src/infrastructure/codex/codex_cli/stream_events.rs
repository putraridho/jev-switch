//! Codex `item.*` events and the turn's usage, one match arm per item kind.

use serde_json::{json, Value};

use super::CodexBackend;
use crate::infrastructure::claude::claude_cli::tool_summary::tool_summary;
use crate::{Event, Usage};

impl CodexBackend {
    /// Messages are remembered (the last one is the answer) and shown; file changes
    /// are shown. `error` items are warnings such as "model metadata not found" and
    /// do not stop the turn, so they are ignored.
    pub(super) fn completed_events(&self, item: &Value) -> Vec<Event> {
        match item["type"].as_str() {
            Some("agent_message") => {
                let text = item["text"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    return vec![];
                }
                self.state().answer = text.to_string();
                vec![Event::Text(text.to_string())]
            }
            Some("file_change") => file_change_events(item),
            Some("reasoning") => {
                let text = item["text"].as_str().unwrap_or("").trim();
                if text.is_empty() { vec![] } else { vec![Event::Thinking(text.to_string())] }
            }
            Some("command_execution") => {
                let ok = item["exit_code"].as_i64().is_none_or(|c| c == 0) && item["status"] != "failed";
                let text = item["aggregated_output"].as_str().unwrap_or("").to_string();
                vec![Event::ToolResult { ok, text }]
            }
            _ => vec![],
        }
    }
}

/// `/usr/bin/bash -lc 'cat a.txt'` is noise; the user wants `cat a.txt`.
pub(super) fn shell_script(command: &str) -> &str {
    let Some((_, script)) = command.split_once(" -lc ") else { return command };
    ['\'', '"']
        .iter()
        .find_map(|q| script.strip_prefix(*q).and_then(|s| s.strip_suffix(*q)))
        .unwrap_or(script)
}

pub(super) fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Codex's `input_tokens` already includes the cached share, which is the scale
/// [`Usage`] wants. It sums every request of the turn, so it is the context size only
/// when `last_request` (the final request alone, from the rollout) is unknown.
pub(super) fn usage_of(u: &Value, last_request: Option<u64>) -> Usage {
    Usage {
        input_tokens: count(u, "input_tokens"),
        cached_input_tokens: count(u, "cached_input_tokens"),
        output_tokens: count(u, "output_tokens"),
        context_tokens: Some(last_request.unwrap_or_else(|| count(u, "input_tokens"))),
    }
}

/// One `edit <path>` per changed file. A failed patch changed nothing, so it is not
/// reported as an edit.
pub(super) fn file_change_events(item: &Value) -> Vec<Event> {
    if item["status"] == "failed" {
        return vec![];
    }
    let changes = item["changes"].as_array().into_iter().flatten();
    changes.filter_map(|c| c["path"].as_str()).map(|p| Event::Tool(tool_summary("edit", &json!({"path": p})))).collect()
}

/// Commands are announced when they start, so the user sees what is running.
pub(super) fn started_events(item: &Value) -> Vec<Event> {
    match item["type"].as_str() {
        Some("command_execution") => {
            let script = shell_script(item["command"].as_str().unwrap_or(""));
            vec![Event::Tool(tool_summary("shell", &json!({"command": script})))]
        }
        _ => vec![],
    }
}

pub(super) fn failure_message(ev: &Value) -> String {
    match ev["error"]["message"].as_str() {
        Some(m) if !m.trim().is_empty() => m.to_string(),
        _ => "codex reported an error".to_string(),
    }
}
