//! The request-body side of the cache-break diagnostics: fingerprint an Anthropic request
//! body for [`PrefixWatch`] and write what it finds as the usage log's `prefix` JSON.
//!
//! `cache_control` markers are ignored: the client moves them every request and they
//! are not part of the cached content.

use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{json, Value};

use crate::gateway::domain::prefix_fingerprint::{BlockFp, Divergence, MessageFp, PrefixDiff, PrefixWatch, RequestFp};

/// Characters of text shown for a differing block.
const SNIPPET: usize = 80;

fn hash_value(v: &Value, h: &mut DefaultHasher) {
    match v {
        Value::Null => 0u8.hash(h),
        Value::Bool(b) => b.hash(h),
        Value::Number(n) => n.to_string().hash(h),
        Value::String(s) => s.hash(h),
        Value::Array(a) => {
            a.len().hash(h);
            a.iter().for_each(|x| hash_value(x, h));
        }
        Value::Object(o) => {
            for (k, x) in o.iter().filter(|(k, _)| k.as_str() != "cache_control") {
                k.hash(h);
                hash_value(x, h);
            }
        }
    }
}

fn hash_of(v: Option<&Value>) -> u64 {
    let mut h = DefaultHasher::new();
    if let Some(v) = v {
        hash_value(v, &mut h);
    }
    h.finish()
}

/// A message's content as a list of blocks (a plain string counts as one text block).
fn blocks(msg: &Value) -> Vec<Value> {
    match msg.get("content") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => vec![json!({ "type": "text", "text": s })],
        _ => Vec::new(),
    }
}

fn block_fp(b: &Value) -> BlockFp {
    let kind = b.get("type").and_then(Value::as_str).unwrap_or("?");
    let text = match kind {
        "text" => b.get("text").and_then(Value::as_str),
        "thinking" => b.get("thinking").and_then(Value::as_str),
        _ => None,
    };
    BlockFp {
        hash: hash_of(Some(b)),
        kind: kind.to_string(),
        bytes: b.to_string().len(),
        text: text.map(|t| t.chars().take(SNIPPET).collect::<String>()),
    }
}

fn fingerprint(req: &Value) -> RequestFp {
    let messages: &[Value] = req.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    RequestFp {
        system: hash_of(req.get("system")),
        tools: hash_of(req.get("tools")),
        msgs: messages
            .iter()
            .map(|m| MessageFp {
                hash: hash_of(Some(m)),
                role: m.get("role").and_then(Value::as_str).map(str::to_string),
                blocks: blocks(m).iter().map(block_fp).collect(),
            })
            .collect(),
    }
}

fn block_json(b: &BlockFp) -> Value {
    json!({ "type": b.kind, "bytes": b.bytes, "text": b.text })
}

fn divergence_json(d: &Divergence) -> Value {
    json!({
        "role": d.role,
        "blocks": d.blocks,
        "prev_blocks": d.prev_blocks,
        "block_index": d.block_index,
        "block": d.block.as_ref().map(block_json),
    })
}

fn diff_json(d: &PrefixDiff) -> Value {
    json!({
        "intact": d.intact,
        "tail_only": d.tail_only,
        "system_changed": d.system_changed,
        "tools_changed": d.tools_changed,
        "prev_messages": d.prev_messages,
        "shared_messages": d.shared_messages,
        "first_diff": d.first_diff.as_ref().map(divergence_json),
    })
}

/// Compare `req` with the earlier request of this session and class that shares the most
/// prefix, remember it, and describe the difference. Returns `Null` for a request with no
/// comparable predecessor.
pub fn observe_request(watch: &PrefixWatch, session: &str, class: &str, req: &Value) -> Value {
    watch.observe(session, class, fingerprint(req)).map(|d| diff_json(&d)).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(system: &str, msgs: Vec<Value>) -> Value {
        json!({ "system": system, "tools": [{ "name": "Bash" }], "messages": msgs })
    }
    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
    }
    fn marked(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text, "cache_control": { "type": "ephemeral" } }] })
    }

    /// The usage log's `prefix` field is what `observe_request` returns, written as `to_string`.
    #[test]
    fn golden_usage_prefix() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("gateway/usage_prefix.txt");
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let watch = PrefixWatch::new();
        let mut out = String::new();
        let mut step = |name: &str, session: &str, class: &str, text: &str| {
            out.push_str(&format!("=== {name}\n{}\n", observe_request(&watch, session, class, &parse(text))));
        };
        // Two requests that differ only in where `cache_control` sits and in key order: intact.
        step("first_has_no_predecessor", "s1", "main", r#"{"system":"sys","tools":[{"name":"Bash","description":"d"}],"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#);
        step("cache_control_moved_and_keys_reordered", "s1", "main", r#"{"messages":[{"content":[{"text":"a","type":"text"}],"role":"user"},{"role":"user","content":[{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]}],"tools":[{"description":"d","name":"Bash"}],"system":"sys"}"#);
        // Every optional part present: an earlier message edited, with a long text and a non-text block.
        let long = "x".repeat(100);
        let before = format!(r#"{{"system":"sys","tools":[],"messages":[{{"role":"user","content":[{{"type":"text","text":"{long}"}}]}},{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{}}}}]}},{{"role":"user","content":"tail"}}]}}"#);
        let after = format!(r#"{{"system":"sys","tools":[],"messages":[{{"role":"user","content":[{{"type":"text","text":"{long}!"}}]}},{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{}}}}]}},{{"role":"user","content":"tail"}},{{"role":"user","content":"new"}}]}}"#);
        step("edited_early_message_first", "s2", "subagent", &before);
        step("edited_early_message", "s2", "subagent", &after);
        // System and tools changed, and the other class and session are separate chains.
        step("system_and_tools_changed", "s2", "subagent", r#"{"system":"sys2","tools":[{"name":"T"}],"messages":[]}"#);
        step("other_class_has_no_predecessor", "s2", "main", r#"{"system":"sys","messages":[]}"#);
        let actual = out;
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn first_request_has_no_predecessor() {
        let w = PrefixWatch::new();
        assert!(observe_request(&w, "s", "main", &req("sys", vec![user("a")])).is_null());
    }

    #[test]
    fn appending_messages_keeps_the_prefix_even_if_cache_control_moves() {
        let w = PrefixWatch::new();
        observe_request(&w, "s", "main", &req("sys", vec![marked("a")]));
        let d = observe_request(&w, "s", "main", &req("sys", vec![user("a"), marked("b")]));
        assert_eq!(d["intact"], true);
        assert_eq!(d["shared_messages"], 1);
    }

    #[test]
    fn an_edited_early_message_is_reported_with_its_position() {
        let w = PrefixWatch::new();
        observe_request(&w, "s", "main", &req("sys", vec![user("<system-reminder> tools: A"), user("b")]));
        let d = observe_request(&w, "s", "main", &req("sys", vec![user("<system-reminder> tools: A, B"), user("b"), user("c")]));
        assert_eq!(d["intact"], false);
        assert_eq!(d["shared_messages"], 0);
        assert_eq!(d["first_diff"]["block"]["type"], "text");
        assert!(d["first_diff"]["block"]["text"].as_str().unwrap().contains("tools: A, B"));
    }

    #[test]
    fn a_rewritten_last_message_is_a_tail_change_not_a_break() {
        let w = PrefixWatch::new();
        observe_request(&w, "s", "main", &req("sys", vec![user("a"), user("b <reminder>")]));
        let d = observe_request(&w, "s", "main", &req("sys", vec![user("a"), user("b"), user("c")]));
        assert_eq!(d["intact"], false);
        assert_eq!(d["tail_only"], true);
        // An edit further back is a real break.
        let w = PrefixWatch::new();
        observe_request(&w, "s", "main", &req("sys", vec![user("a"), user("b"), user("c")]));
        let d = observe_request(&w, "s", "main", &req("sys", vec![user("a2"), user("b"), user("c"), user("d")]));
        assert_eq!(d["tail_only"], false);
    }

    #[test]
    fn system_and_tool_changes_are_flagged_separately() {
        let w = PrefixWatch::new();
        observe_request(&w, "s", "main", &req("sys", vec![user("a")]));
        let d = observe_request(&w, "s", "main", &req("sys2", vec![user("a"), user("b")]));
        assert_eq!(d["system_changed"], true);
        assert_eq!(d["tools_changed"], false);
    }

    #[test]
    fn parallel_chains_match_their_own_predecessor() {
        let w = PrefixWatch::new();
        observe_request(&w, "s", "subagent", &req("sys", vec![user("x1")]));
        observe_request(&w, "s", "subagent", &req("sys", vec![user("y1")]));
        let d = observe_request(&w, "s", "subagent", &req("sys", vec![user("x1"), user("x2")]));
        assert_eq!(d["intact"], true);
    }

    #[test]
    fn sessions_do_not_mix() {
        let w = PrefixWatch::new();
        observe_request(&w, "s1", "main", &req("sys", vec![user("a")]));
        assert!(observe_request(&w, "s2", "main", &req("sys", vec![user("a")])).is_null());
    }
}
