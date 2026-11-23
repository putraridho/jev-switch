//! Gemini CLI `stream-json` events, one match arm per event kind.

use serde_json::Value;

use super::{GeminiBackend, Pending, EMPTY_ANSWER};
use crate::infrastructure::claude::claude_cli::tool_summary::tool_summary;
use crate::infrastructure::usage_measurement::measurement_of;
use crate::{Event, Finish, Usage};

pub(super) fn uint(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Gemini's `input_tokens` is the prompt count, which already includes the cached
/// share (`cached`), so it maps straight onto [`Usage`].
fn gemini_usage(stats: &Value) -> Usage {
    Usage {
        input_tokens: uint(stats, "input_tokens"),
        cached_input_tokens: uint(stats, "cached"),
        output_tokens: uint(stats, "output_tokens"),
        context_tokens: None,
        measurement: measurement_of(stats),
    }
}

/// The shared success rule. Antigravity can report `SUCCESS` with an empty
/// response when it did nothing (google-antigravity/antigravity-cli#1065) and
/// Gemini CLI can end a turn with no text, so a blank answer is a failure, never a
/// `Finished` that the caller would show as a reply.
pub fn finish_or_empty(text: &str, session_id: Option<String>, usage: Usage, note: &str) -> Event {
    if text.trim().is_empty() {
        return Event::Failed { message: format!("{EMPTY_ANSWER}{note}") };
    }
    Event::Finished(Finish { text: text.to_string(), session_id, usage })
}

/// A tool that failed, shown as a hint: in headless mode a denied tool is the
/// usual reason a turn goes nowhere, and nothing else tells the user.
pub(super) fn tool_failure(message: Option<&str>) -> Option<Event> {
    let message = message?.trim();
    (!message.is_empty()).then(|| Event::Hint(format!("\u{2717} tool failed: {}", message.chars().take(200).collect::<String>())))
}

impl GeminiBackend {
    /// Gemini CLI `--output-format stream-json`: `init`, `message`, `tool_use`,
    /// `tool_result`, `error` and a final `result`.
    pub(super) fn gemini_events(&self, ev: &Value) -> Vec<Event> {
        let mut pending = self.pending();
        match ev["type"].as_str() {
            Some("init") => {
                *pending = Pending { session_id: ev["session_id"].as_str().map(String::from), ..Pending::default() };
                vec![]
            }
            Some("message") => {
                if ev["role"] == "assistant" {
                    let text = ev["content"].as_str().unwrap_or("");
                    pending.text.push_str(text);
                    // `delta` messages are pieces of the text being written; the whole text
                    // still comes at the next tool call or in the result.
                    if ev["delta"] == true && !text.is_empty() {
                        return vec![Event::TextDelta(text.to_string())];
                    }
                }
                vec![]
            }
            Some("tool_use") => {
                let tool = tool_summary(ev["tool_name"].as_str().unwrap_or("?"), &ev["parameters"]);
                // Text before a tool call is narration; the answer is what follows the last one.
                let narration = std::mem::take(&mut pending.text);
                let narration = narration.trim();
                let said = (!narration.is_empty()).then(|| Event::Text(narration.to_string()));
                said.into_iter().chain([Event::Tool(tool)]).collect()
            }
            Some("tool_result") if ev["status"] == "error" => {
                tool_failure(ev["error"]["message"].as_str().or(ev["output"].as_str())).into_iter().collect()
            }
            Some("error") => {
                let message = ev["message"].as_str().unwrap_or("").to_string();
                if ev["severity"] == "error" {
                    pending.notice = Some(message.clone());
                }
                vec![Event::Text(format!("! {message}"))]
            }
            Some("result") => vec![gemini_result(ev, std::mem::take(&mut *pending))],
            _ => vec![],
        }
    }
}

/// `error.message` exists only when the CLI died with an exception; a failed
/// generation (empty reply, safety block) leaves just the earlier `error` event.
fn gemini_result(ev: &Value, pending: Pending) -> Event {
    if ev["status"] != "success" {
        let message = ev["error"]["message"].as_str().map(String::from).or(pending.notice);
        return Event::Failed { message: message.unwrap_or_else(|| "gemini reported an error".into()) };
    }
    finish_or_empty(&pending.text, pending.session_id, gemini_usage(&ev["stats"]), "")
}
