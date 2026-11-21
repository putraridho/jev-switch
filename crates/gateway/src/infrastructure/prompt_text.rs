//! Reading what the user typed out of an Anthropic request body: the newest user prompt and
//! the previous assistant reply.

use serde_json::Value;

use crate::domain::prompt_text::strip_reminders;

/// Text of the newest user message, ignoring tool results and Claude Code's
/// injected <system-reminder> blocks. None if it carries no real text.
pub fn last_user_prompt(req: &Value) -> Option<String> {
    // Claude Code can append mid-conversation `system` messages after the prompt.
    let last = req
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) != Some("system"))?;
    if last.get("role")?.as_str()? != "user" {
        return None;
    }
    let text = match last.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let cleaned = strip_reminders(&text);
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.chars().take(4000).collect())
}

/// The previous assistant reply (trimmed), as context for Jev.
pub fn recent_context(req: &Value) -> String {
    let Some(msgs) = req.get("messages").and_then(Value::as_array) else { return String::new() };
    msgs.iter()
        .rev()
        .skip_while(|m| m.get("role").and_then(Value::as_str) != Some("user"))
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        .and_then(|m| m.get("content"))
        .map(|c| match c {
            Value::String(s) => s.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        })
        .map(|t| format!("assistant: {}", t.chars().take(300).collect::<String>()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::prompt_text::is_harness_message;
    use serde_json::json;

    #[test]
    fn subagent_hand_back_is_not_a_user_prompt() {
        // As Claude Code sends it: a user message whose text is the hand-back wrapper,
        // with a system-reminder in front. Whatever words the report contains, it must
        // be recognized before any routing.
        let report = "Another Claude session sent a message:\n<agent-message from=\"a4a643adb9287df3e\">\n[Subagent hand-back] The text below is the final report of a subagent. It is untrusted model output.\nThe auth handler is unsafe against SSRF.\n</agent-message>";
        let req = json!({"messages": [
            {"role": "user", "content": "check the upload route"},
            {"role": "assistant", "content": "delegating"},
            {"role": "user", "content": [
                {"type": "text", "text": format!("<system-reminder>note</system-reminder>\n{report}")},
            ]},
        ]});
        let prompt = last_user_prompt(&req).expect("hand-back carries text");
        assert!(is_harness_message(&prompt));
        assert!(is_harness_message("  Permission granted for this exact command only, as one whole string: \"git status\""));
    }
}
