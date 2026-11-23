//! Parsing of `streaming-messages-json` lines into events.

use serde_json::Value;

use crate::infrastructure::claude::claude_cli::tool_summary::tool_summary;
use crate::infrastructure::usage_measurement::measurement_of;
use crate::{Event, Finish, Usage};

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Grok's `input_tokens` is the uncached share only, so the total adds both cache
/// buckets. The context size is not reported per call here, hence `None`.
fn usage_of(result: &Value) -> Usage {
    let u = &result["usage"];
    let cached = count(u, "cache_read_input_tokens");
    Usage {
        input_tokens: count(u, "input_tokens") + cached + count(u, "cache_creation_input_tokens"),
        cached_input_tokens: cached,
        output_tokens: count(u, "output_tokens"),
        context_tokens: None,
        measurement: measurement_of(u),
    }
}

fn assistant_events(ev: &Value) -> Vec<Event> {
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter_map(|b| match b["type"].as_str() {
            Some("tool_use") => Some(Event::Tool(tool_summary(b["name"].as_str().unwrap_or("?"), &b["input"]))),
            Some("text") => {
                let t = b["text"].as_str().unwrap_or("").trim();
                (!t.is_empty()).then(|| Event::Text(t.to_string()))
            }
            _ => None,
        })
        .collect()
}

/// Why a `result` failed: its `errors[]`, else its text, else its subtype.
fn failure_message(ev: &Value) -> String {
    let errors: Vec<String> = ev["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| e.as_str().map_or_else(|| e.to_string(), String::from))
        .collect();
    let text = ev["result"].as_str().unwrap_or("").trim();
    if !errors.is_empty() {
        errors.join("; ")
    } else if !text.is_empty() {
        text.to_string()
    } else {
        ev["subtype"].as_str().map_or_else(|| "grok reported an error".into(), |s| format!("grok failed: {s}"))
    }
}

fn result_event(ev: &Value) -> Event {
    if ev["is_error"].as_bool() == Some(true) {
        return Event::Failed { message: failure_message(ev) };
    }
    Event::Finished(Finish {
        text: ev["result"].as_str().unwrap_or("").to_string(),
        session_id: ev["session_id"].as_str().map(String::from),
        usage: usage_of(ev),
    })
}

pub(super) fn parse_line(line: &str) -> Vec<Event> {
    let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
    match ev["type"].as_str() {
        Some("assistant") => assistant_events(&ev),
        Some("result") => vec![result_event(&ev)],
        // Documented for every output format, e.g. "Couldn't start session: ...".
        Some("error") => vec![Event::Failed { message: ev["message"].as_str().unwrap_or("grok reported an error").into() }],
        _ => vec![],
    }
}
