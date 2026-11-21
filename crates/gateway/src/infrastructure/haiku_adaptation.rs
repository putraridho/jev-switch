//! Stripping from a request body what Haiku rejects.

use serde_json::Value;

/// A 1M-context session makes Claude Code add settings Haiku 4.5 rejected: the
/// long-context beta, adaptive thinking, effort, and mid-conversation system
/// messages. Haiku 5.5 accepted adaptive thinking, effort and context management
/// when Claude Code sent them directly (18 logged requests, all 200), but stripping
/// them from a title or summary request costs nothing, and the Sonnet fallback in
/// `forward` covers anything it still rejects.
pub fn adapt_for_haiku(req: &mut Value) {
    let Some(o) = req.as_object_mut() else { return };
    if let Some(m) = o.get_mut("messages").and_then(Value::as_array_mut) {
        m.retain(|m| m.get("role").and_then(Value::as_str) != Some("system"));
    }
    if o.get("thinking").and_then(|t| t.get("type")).and_then(Value::as_str) == Some("adaptive") {
        o.remove("thinking");
        o.remove("context_management"); // its edits only apply with thinking
    }
    if let Some(cfg) = o.get_mut("output_config").and_then(Value::as_object_mut) {
        cfg.remove("effort");
    }
    if o.get("output_config").and_then(Value::as_object).is_some_and(|c| c.is_empty()) {
        o.remove("output_config");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_catalog::domain::model_table::MODEL_CHEAP;
    use serde_json::json;

    #[test]
    fn haiku_request_loses_what_haiku_rejects() {
        let mut req = json!({
            "model": MODEL_CHEAP, "max_tokens": 5,
            "thinking": {"type": "adaptive"}, "context_management": {"edits": []},
            "output_config": {"effort": "high"},
            "messages": [{"role": "user", "content": "a"}, {"role": "system", "content": "note"}, {"role": "user", "content": "b"}],
        });
        adapt_for_haiku(&mut req);
        assert_eq!(req["messages"].as_array().unwrap().len(), 2);
        for gone in ["thinking", "context_management", "output_config"] {
            assert!(req.get(gone).is_none(), "{gone} should be removed");
        }
        assert_eq!(req["max_tokens"], 5);
    }

    #[test]
    fn explicit_thinking_budget_and_other_output_config_survive() {
        let mut req = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}, "output_config": {"effort": "low", "format": {}}});
        adapt_for_haiku(&mut req);
        assert_eq!(req["thinking"]["budget_tokens"], 1024);
        assert_eq!(req["output_config"], json!({"format": {}}));
    }
}
