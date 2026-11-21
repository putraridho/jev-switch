//! Claude Code's stream-json output, one event kind at a time.

use serde_json::Value;

use super::tool_summary::tool_summary;
use crate::backend::{Event, Finish, PlanUsage, Usage};
use crate::shared::domain::limit_notice::is_limit_message;

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Everything sent in one call: fresh input plus cache reads plus cache writes.
fn total_input(v: &Value) -> u64 {
    count(v, "input_tokens") + count(v, "cache_read_input_tokens") + count(v, "cache_creation_input_tokens")
}

/// `usage` sums every API call in the turn; the context size is the last call's
/// input, so it comes from the final `iterations` entry.
fn usage_of(result: &Value) -> Usage {
    let u = &result["usage"];
    let last = u["iterations"].as_array().and_then(|a| a.last()).unwrap_or(u);
    Usage {
        input_tokens: total_input(u),
        cached_input_tokens: count(u, "cache_read_input_tokens"),
        output_tokens: count(u, "output_tokens"),
        context_tokens: Some(total_input(last)),
    }
}

pub(super) fn assistant_events(ev: &Value) -> Vec<Event> {
    let nested = !ev["parent_tool_use_id"].is_null();
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    let mut out = Vec::new();
    for b in blocks {
        match b["type"].as_str() {
            Some("tool_use") => {
                let name = b["name"].as_str().unwrap_or("?");
                out.push(Event::Tool(tool_summary(name, &b["input"])));
                if !nested {
                    out.extend(edit_event(name, &b["input"]));
                }
            }
            Some("text") if !nested => {
                let t = b["text"].as_str().unwrap_or("").trim();
                // The limit notice is reported as an error by the runner, not shown as an answer.
                if !t.is_empty() && !is_limit_message(t) {
                    out.push(Event::Text(t.to_string()));
                }
            }
            Some("thinking") if !nested => {
                let t = b["thinking"].as_str().unwrap_or("").trim();
                if !t.is_empty() {
                    out.push(Event::Thinking(t.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// A piece of the answer as Claude writes it (`--include-partial-messages`). Only top-level
/// text deltas: a subagent's text, thinking and tool input stay out. The whole message still
/// arrives as `assistant` and is what the turn commits.
pub(super) fn stream_delta(ev: &Value) -> Vec<Event> {
    if !ev["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    let inner = &ev["event"];
    if inner["type"] != "content_block_delta" || inner["delta"]["type"] != "text_delta" {
        return Vec::new();
    }
    let text = inner["delta"]["text"].as_str().unwrap_or("");
    (!text.is_empty()).then(|| Event::TextDelta(text.to_string())).into_iter().collect()
}

fn lines_of(v: &Value) -> Vec<String> {
    v.as_str().map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
}

/// What an Edit, MultiEdit or Write call changes, from its input.
fn edit_event(tool: &str, input: &Value) -> Option<Event> {
    let path = input["file_path"].as_str()?.to_string();
    match tool {
        "Edit" => Some(Event::Edit { path, removed: lines_of(&input["old_string"]), added: lines_of(&input["new_string"]) }),
        "MultiEdit" => {
            let edits = input["edits"].as_array()?;
            let removed = edits.iter().flat_map(|e| lines_of(&e["old_string"])).collect();
            let added = edits.iter().flat_map(|e| lines_of(&e["new_string"])).collect();
            Some(Event::Edit { path, removed, added })
        }
        "Write" => Some(Event::Edit { path, removed: Vec::new(), added: lines_of(&input["content"]) }),
        _ => None,
    }
}

/// The text of a `tool_result` block: a string, or a list of text parts.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Claude Code wraps a failed call's message in `<tool_use_error>…</tool_use_error>`, markup
/// meant for the model; the person reading the tool row wants the message alone.
fn without_error_markup(text: String) -> String {
    let t = text.trim();
    match t.strip_prefix("<tool_use_error>") {
        Some(inner) => inner.strip_suffix("</tool_use_error>").unwrap_or(inner).trim().to_string(),
        None => text,
    }
}

/// Tool results of the main conversation (a sub-agent's are its own business).
pub(super) fn user_events(ev: &Value) -> Vec<Event> {
    if !ev["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    // `--replay-user-messages`: a message from stdin, echoed when the model has it.
    if ev["isReplay"].as_bool() == Some(true) {
        let content = &ev["message"]["content"];
        let text = match content.as_str() {
            Some(text) => text.to_string(),
            None => content.as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        };
        return vec![Event::Steered(text)];
    }
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter(|b| b["type"] == "tool_result")
        .map(|b| Event::ToolResult { ok: b["is_error"].as_bool() != Some(true), text: without_error_markup(result_text(&b["content"])) })
        .collect()
}

pub(super) fn result_event(ev: &Value) -> Event {
    let text = ev["result"].as_str().unwrap_or("").to_string();
    if ev["is_error"].as_bool() == Some(true) {
        let message = if text.trim().is_empty() { "claude reported an error".to_string() } else { text };
        return Event::Failed { message };
    }
    Event::Finished(Finish { text, session_id: ev["session_id"].as_str().map(String::from), usage: usage_of(ev) })
}

/// The busiest plan window (highest utilization) from a `rate_limit_event`.
pub(super) fn plan_of(ev: &Value) -> Option<PlanUsage> {
    let windows = ev["rate_limit_info"]["unifiedWindows"].as_object()?;
    let (name, w) = windows
        .iter()
        .filter_map(|(k, w)| Some((k, w, w["utilization"].as_f64()?)))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(k, w, _)| (k, w))?;
    Some(PlanUsage {
        window: name.clone(),
        utilization: w["utilization"].as_f64()?,
        resets_at: w["resetsAt"].as_u64(),
    })
}
