//! Streamed `session/update` messages: answer text, tool rows and usage.

use serde_json::{json, Value};

use super::tool_row::tool_row_summary;
use super::Acp;
use crate::Event;

impl Acp {
    pub(super) fn update(&mut self, params: &Value) -> Vec<Event> {
        if !self.active || params["sessionId"].as_str() != self.session.as_deref() {
            return vec![];
        }
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                let text = update["content"]["text"].as_str().unwrap_or("");
                self.text.push_str(text);
                // The answer as it is written; the finished turn still reports the whole.
                if !text.is_empty() {
                    return vec![Event::TextDelta(text.to_string())];
                }
            }
            Some("tool_call" | "tool_call_update") => {
                let Some(id) = update["toolCallId"].as_str() else { return vec![] };
                let call = self.calls.entry(id.into()).or_insert_with(|| json!({}));
                if let Some(fields) = update.as_object() {
                    for (k, v) in fields {
                        call[k] = v.clone();
                    }
                }
                // Antigravity's first message about a call often has only its name; what it
                // runs comes in an update. The row waits for that, or for the call to end.
                if !self.announced.contains(id) {
                    let ended = matches!(call["status"].as_str(), Some("completed" | "failed"));
                    let name = call["name"].as_str().or(call["title"].as_str()).unwrap_or("tool").to_string();
                    if let Some(summary) = tool_row_summary(call).or_else(|| ended.then_some(name)) {
                        self.announced.insert(id.into());
                        return vec![Event::Tool(summary)];
                    }
                }
            }
            Some("usage_update") => {
                if let Some(used) = update["used"].as_u64() {
                    self.usage.context_tokens = Some(used);
                }
                // The protocol's `size` is the agent's own context window.
                if let Some(size) = update["size"].as_u64() {
                    self.usage.window_tokens = crate::Usage::narrow(Some(size));
                }
            }
            _ => {}
        }
        vec![]
    }
}
