//! One-line summaries of tool calls, shared by every provider that streams tool use.

use serde_json::Value;

/// The input field that says most about a tool call: a path, a command, a pattern.
pub fn telling_field(input: &Value) -> Option<&str> {
    ["file_path", "command", "pattern", "path", "url", "description", "brief", "worker_id"].iter().find_map(|k| input.get(*k).and_then(Value::as_str))
}

/// One-line summary of a tool call: its name plus the most telling input field.
pub fn tool_summary(name: &str, input: &Value) -> String {
    match telling_field(input) {
        Some(f) => format!("{name} {}", f.chars().take(100).collect::<String>()),
        None => name.to_string(),
    }
}
