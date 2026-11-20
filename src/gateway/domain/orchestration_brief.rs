//! The jev-orchestrate brief a mod attaches to a prompt that triggered orchestration.
//! The gateway only reads it; it never edits the request.

use serde_json::Value;

/// Start of the context block the jev-orchestrate mod attaches to a prompt that
/// triggered orchestration.
const ORCH_MARKER: &str = "<jev-orchestrate>";

/// Whether the turn that this request starts carries the jev-orchestrate brief: the
/// marker in any user message after the last assistant one, reminders included (the
/// brief may arrive wrapped in one, or as its own block or message).
pub(crate) fn is_orchestrated(req: &Value) -> bool {
    let Some(msgs) = req.get("messages").and_then(Value::as_array) else { return false };
    msgs.iter()
        .rev()
        .take_while(|m| m.get("role").and_then(Value::as_str) != Some("assistant"))
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        .any(|m| match m.get("content") {
            Some(Value::String(s)) => s.contains(ORCH_MARKER),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .any(|t| t.contains(ORCH_MARKER)),
            _ => false,
        })
}

/// The prompt without the orchestration brief, for the decision log.
pub(crate) fn strip_orchestrate_block(prompt: &str) -> String {
    match prompt.find(ORCH_MARKER) {
        Some(i) => {
            let end = prompt[i..].find("</jev-orchestrate>").map(|e| i + e + "</jev-orchestrate>".len()).unwrap_or(prompt.len());
            format!("{}{}", &prompt[..i], &prompt[end..]).trim().to_string()
        }
        None => prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn orchestration_marker_is_found_wherever_the_brief_lands() {
        let brief = "<jev-orchestrate>plan, delegate, review</jev-orchestrate>";
        let before = json!([{"role": "user", "content": "old task"}, {"role": "assistant", "content": "done"}]);
        let with = |tail: Value| {
            let mut m = before.as_array().unwrap().clone();
            m.extend(tail.as_array().unwrap().iter().cloned());
            json!({ "messages": m })
        };
        // As its own text block, wrapped in a reminder, or as a separate user message.
        assert!(is_orchestrated(&with(json!([{"role": "user", "content": [{"type": "text", "text": "build X"}, {"type": "text", "text": brief}]}]))));
        assert!(is_orchestrated(&with(json!([{"role": "user", "content": format!("build X\n<system-reminder>{brief}</system-reminder>")}]))));
        assert!(is_orchestrated(&with(json!([{"role": "user", "content": "build X"}, {"role": "user", "content": brief}]))));
        // Not triggered, or only an earlier turn was.
        assert!(!is_orchestrated(&with(json!([{"role": "user", "content": "build X"}]))));
        let earlier = json!({ "messages": [
            {"role": "user", "content": format!("old task {brief}")},
            {"role": "assistant", "content": "done"},
            {"role": "user", "content": "thanks"},
        ]});
        assert!(!is_orchestrated(&earlier));
    }

    #[test]
    fn the_brief_is_left_out_of_the_logged_prompt() {
        assert_eq!(strip_orchestrate_block("build X\n<jev-orchestrate>plan</jev-orchestrate>"), "build X");
        assert_eq!(strip_orchestrate_block("build X"), "build X");
    }
}
