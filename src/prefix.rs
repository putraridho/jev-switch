//! Cache-break diagnostics: where did this request's prefix first differ from the
//! closest earlier request of the same session and class?
//!
//! Anthropic's prompt cache matches on the request prefix (tools, system, then
//! messages in order). A cache read shorter than the previous request's context
//! means something early in the prefix changed. The usage log has token counts but
//! not content, so this module fingerprints each request and, when the prefix
//! diverges, records what kind of block moved. Only hashes are kept in memory and
//! only a short description of the first differing block reaches the log.
//!
//! `cache_control` markers are ignored: the client moves them every request and they
//! are not part of the cached content.

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;

use serde_json::{json, Value};

/// Earlier requests remembered per session and class. Parallel subagents interleave,
/// so one chain is not enough; more than this and the match gets slow and noisy.
const KEEP: usize = 12;
/// Characters of text shown for a differing block.
const SNIPPET: usize = 80;

struct Fp {
    system: u64,
    tools: u64,
    msgs: Vec<u64>,
    /// Per message, the hash of each content block (to find the block that changed).
    blocks: Vec<Vec<u64>>,
}

#[derive(Default)]
pub struct PrefixWatch {
    recent: Mutex<HashMap<String, VecDeque<Fp>>>,
}

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

fn fingerprint(req: &Value) -> Fp {
    let messages: &[Value] = req.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    Fp {
        system: hash_of(req.get("system")),
        tools: hash_of(req.get("tools")),
        msgs: messages.iter().map(|m| hash_of(Some(m))).collect(),
        blocks: messages.iter().map(|m| blocks(m).iter().map(|b| hash_of(Some(b))).collect()).collect(),
    }
}

/// A message's content as a list of blocks (a plain string counts as one text block).
fn blocks(msg: &Value) -> Vec<Value> {
    match msg.get("content") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => vec![json!({ "type": "text", "text": s })],
        _ => Vec::new(),
    }
}

fn describe_block(b: &Value) -> Value {
    let kind = b.get("type").and_then(Value::as_str).unwrap_or("?");
    let text = match kind {
        "text" => b.get("text").and_then(Value::as_str),
        "thinking" => b.get("thinking").and_then(Value::as_str),
        _ => None,
    };
    json!({
        "type": kind,
        "bytes": b.to_string().len(),
        "text": text.map(|t| t.chars().take(SNIPPET).collect::<String>()),
    })
}

/// The message at the divergence point: its role and the first block that differs
/// from the earlier request's message at the same position (if there was one).
fn describe_message(msg: Option<&Value>, old_blocks: Option<&Vec<u64>>) -> Value {
    let Some(msg) = msg else { return Value::Null };
    let mine = blocks(msg);
    let at = (0..mine.len()).find(|&j| old_blocks.and_then(|o| o.get(j)) != Some(&hash_of(mine.get(j))));
    json!({
        "role": msg.get("role"),
        "blocks": mine.len(),
        "prev_blocks": old_blocks.map(Vec::len),
        "block_index": at,
        "block": at.and_then(|j| mine.get(j)).map(describe_block),
    })
}

impl PrefixWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compare `req` with the earlier request of this session and class that shares
    /// the most prefix, remember it, and describe the difference. Returns `Null` for
    /// a request with no comparable predecessor.
    pub fn observe(&self, session: &str, class: &str, req: &Value) -> Value {
        let fp = fingerprint(req);
        let key = format!("{session}|{class}");
        let mut all = self.recent.lock().unwrap();
        let list = all.entry(key).or_default();

        // Closest predecessor: same system and tools first, then longest shared message prefix.
        let common = |p: &Fp| p.msgs.iter().zip(&fp.msgs).take_while(|(a, b)| a == b).count();
        let best = list.iter().enumerate().max_by_key(|(i, p)| ((p.system == fp.system) as u8 + (p.tools == fp.tools) as u8, common(p), *i));
        let out = best.map(|(_, p)| {
            let n = common(p);
            let (sys, tools) = (p.system != fp.system, p.tools != fp.tools);
            // The prefix is intact when the earlier request is wholly contained in this one.
            let intact = !sys && !tools && n == p.msgs.len();
            let messages = req.get("messages").and_then(Value::as_array);
            let diverged = (!intact && !sys && !tools).then(|| {
                // `n` is where the shared prefix ends: the earlier message there was edited
                // (or the conversation was rewound or compacted and it no longer exists).
                describe_message(messages.and_then(|m| m.get(n)), p.blocks.get(n))
            });
            json!({
                "intact": intact,
                "system_changed": sys,
                "tools_changed": tools,
                "prev_messages": p.msgs.len(),
                "shared_messages": n,
                "first_diff": diverged,
            })
        });

        list.push_back(fp);
        if list.len() > KEEP {
            list.pop_front();
        }
        out.unwrap_or(Value::Null)
    }
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

    #[test]
    fn first_request_has_no_predecessor() {
        let w = PrefixWatch::new();
        assert!(w.observe("s", "main", &req("sys", vec![user("a")])).is_null());
    }

    #[test]
    fn appending_messages_keeps_the_prefix_even_if_cache_control_moves() {
        let w = PrefixWatch::new();
        w.observe("s", "main", &req("sys", vec![marked("a")]));
        let d = w.observe("s", "main", &req("sys", vec![user("a"), marked("b")]));
        assert_eq!(d["intact"], true);
        assert_eq!(d["shared_messages"], 1);
    }

    #[test]
    fn an_edited_early_message_is_reported_with_its_position() {
        let w = PrefixWatch::new();
        w.observe("s", "main", &req("sys", vec![user("<system-reminder> tools: A"), user("b")]));
        let d = w.observe("s", "main", &req("sys", vec![user("<system-reminder> tools: A, B"), user("b"), user("c")]));
        assert_eq!(d["intact"], false);
        assert_eq!(d["shared_messages"], 0);
        assert_eq!(d["first_diff"]["block"]["type"], "text");
        assert!(d["first_diff"]["block"]["text"].as_str().unwrap().contains("tools: A, B"));
    }

    #[test]
    fn system_and_tool_changes_are_flagged_separately() {
        let w = PrefixWatch::new();
        w.observe("s", "main", &req("sys", vec![user("a")]));
        let d = w.observe("s", "main", &req("sys2", vec![user("a"), user("b")]));
        assert_eq!(d["system_changed"], true);
        assert_eq!(d["tools_changed"], false);
    }

    #[test]
    fn parallel_chains_match_their_own_predecessor() {
        let w = PrefixWatch::new();
        w.observe("s", "subagent", &req("sys", vec![user("x1")]));
        w.observe("s", "subagent", &req("sys", vec![user("y1")]));
        let d = w.observe("s", "subagent", &req("sys", vec![user("x1"), user("x2")]));
        assert_eq!(d["intact"], true);
    }

    #[test]
    fn sessions_do_not_mix() {
        let w = PrefixWatch::new();
        w.observe("s1", "main", &req("sys", vec![user("a")]));
        assert!(w.observe("s2", "main", &req("sys", vec![user("a")])).is_null());
    }
}
