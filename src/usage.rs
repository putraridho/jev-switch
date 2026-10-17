//! Usage logging for jev-gateway, as JSON lines for later analysis.
//!
//! - [`UsageTap`] watches one /v1/messages response as it streams to the client
//!   (it only reads; the client gets the bytes unchanged) and writes a request
//!   record: tokens, latency, status, tools the model called, rate-limit headers.
//! - [`Turns`] rolls requests up into one record per user turn and records how the
//!   user reacted to the previous turn (gap, rephrase, model switch), the outcome
//!   signal for judging routing later.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Map, Value};

use crate::log;

/// Cap on the text a tap keeps while parsing; usage fields are tiny, content is not.
const TAP_BUF_MAX: usize = 8 * 1024 * 1024;
/// Token fields summed into turn records.
const TOKEN_FIELDS: [&str; 4] =
    ["input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"];
const CWD_KEY: &str = "Primary working directory: ";

/// Request settings worth analyzing: every top-level field except the content.
/// Large values are replaced by their size.
pub fn request_params(req: &Value) -> Value {
    const SKIP: &[&str] = &["messages", "system", "tools", "model", "stream", "metadata"];
    let mut out = Map::new();
    for (k, v) in req.as_object().into_iter().flatten() {
        if SKIP.contains(&k.as_str()) {
            continue;
        }
        let n = v.to_string().len();
        out.insert(k.clone(), if n > 1024 { json!(format!("<{n} bytes>")) } else { v.clone() });
    }
    Value::Object(out)
}

/// The project directory Claude Code reports, in the system prompt or in a
/// text block of the conversation (after a compaction it is no longer in the first
/// messages). Tool results are not searched: files may quote the key.
pub fn find_cwd(req: &Value) -> Option<String> {
    let in_messages = || req.get("messages")?.as_array()?.iter().find_map(|m| cwd_in(m.get("content")?));
    req.get("system").and_then(cwd_in).or_else(in_messages)
}

fn cwd_in(content: &Value) -> Option<String> {
    let from_text = |t: &str| {
        let line = t[t.find(CWD_KEY)? + CWD_KEY.len()..].lines().next()?.trim();
        (!line.is_empty()).then(|| line.to_string())
    };
    match content {
        Value::String(s) => from_text(s),
        Value::Array(bs) => bs.iter().find_map(|b| b.get("text").and_then(Value::as_str).and_then(from_text)),
        _ => None,
    }
}

pub fn project_of(cwd: &str) -> &str {
    cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(cwd)
}

/// Jaccard overlap of the word sets: near 1.0 means the same question asked again.
pub fn similarity(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
    };
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / union as f64
}

// --- per-request tap ---------------------------------------------------------

/// One usage record per /v1/messages request. Writes a line when the response ends
/// or is dropped, so aborted requests are logged too, with `complete: false`.
pub struct UsageTap {
    path: String,
    record: Map<String, Value>,
    turns: Arc<Turns>,
    session: String,
    prompt_id: Option<String>,
    class: String,
    started: Instant,
    ttfb_ms: Option<u64>,
    sse: bool,
    encoded: Option<String>,
    status: Option<u16>,
    request_id: Option<String>,
    ratelimit: Map<String, Value>,
    /// Unparsed tail of an SSE stream, or the whole body of a JSON response (capped).
    buf: Vec<u8>,
    bytes: u64,
    usage: Map<String, Value>,
    message_id: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    blocks: BTreeMap<String, u32>,
    tools_called: Vec<String>,
    stopped: bool,
    /// Set by the caller when the upstream request itself failed.
    pub error: Option<Value>,
}

impl UsageTap {
    pub fn new(
        path: String,
        started: Instant,
        record: Value,
        turns: Arc<Turns>,
        session: String,
        prompt_id: Option<String>,
        class: String,
    ) -> Self {
        Self {
            path,
            record: record.as_object().cloned().unwrap_or_default(),
            turns,
            session,
            prompt_id,
            class,
            started,
            ttfb_ms: None,
            sse: false,
            encoded: None,
            status: None,
            request_id: None,
            ratelimit: Map::new(),
            buf: Vec::new(),
            bytes: 0,
            usage: Map::new(),
            message_id: None,
            model: None,
            stop_reason: None,
            blocks: BTreeMap::new(),
            tools_called: Vec::new(),
            stopped: false,
            error: None,
        }
    }

    /// Add or replace a field of the request record (e.g. after a fallback retry).
    pub fn set(&mut self, key: &str, value: Value) {
        self.record.insert(key.into(), value);
    }

    pub fn start_response(&mut self, status: u16, headers: &reqwest::header::HeaderMap) {
        let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
        self.status = Some(status);
        self.sse = get("content-type").is_some_and(|t| t.contains("event-stream"));
        self.encoded = get("content-encoding").filter(|e| *e != "identity").map(str::to_string);
        self.request_id = get("request-id").map(str::to_string);
        for (k, v) in headers {
            let name = k.as_str();
            if name.starts_with("anthropic-ratelimit-") || name == "retry-after" {
                if let Ok(v) = v.to_str() {
                    self.ratelimit.insert(name.to_string(), json!(v));
                }
            }
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.ttfb_ms.get_or_insert_with(|| self.started.elapsed().as_millis() as u64);
        self.bytes += chunk.len() as u64;
        if self.encoded.is_some() {
            return; // compressed: can't be read here
        }
        if !self.sse {
            if self.buf.len() + chunk.len() <= TAP_BUF_MAX {
                self.buf.extend_from_slice(chunk);
            }
            return;
        }
        self.buf.extend_from_slice(chunk);
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            if let Some(data) = line.strip_prefix(b"data:") {
                if let Ok(ev) = serde_json::from_slice::<Value>(data.trim_ascii()) {
                    self.event(&ev);
                }
            }
        }
        if self.buf.len() > TAP_BUF_MAX {
            self.buf.clear();
        }
    }

    /// Merge `usage` into the record. `message_delta` repeats and updates the counts.
    fn take_usage(&mut self, u: Option<&Value>) {
        for (k, v) in u.and_then(Value::as_object).into_iter().flatten() {
            if !v.is_null() {
                self.usage.insert(k.clone(), v.clone());
            }
        }
    }

    /// Count a content block by type; remember the names of tools the model called.
    fn take_block(&mut self, b: &Value) {
        let Some(ty) = b["type"].as_str() else { return };
        *self.blocks.entry(ty.to_string()).or_default() += 1;
        if ty.ends_with("tool_use") {
            if let Some(n) = b["name"].as_str() {
                self.tools_called.push(n.to_string());
            }
        }
    }

    fn event(&mut self, ev: &Value) {
        match ev["type"].as_str() {
            Some("message_start") => {
                let m = &ev["message"];
                self.message_id = m["id"].as_str().map(str::to_string);
                self.model = m["model"].as_str().map(str::to_string);
                self.take_usage(m.get("usage"));
            }
            Some("content_block_start") => self.take_block(&ev["content_block"]),
            Some("message_delta") => {
                self.take_usage(ev.get("usage"));
                if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
            }
            Some("message_stop") => self.stopped = true,
            Some("error") => {
                self.error = Some(ev["error"].clone());
                self.stopped = true;
            }
            _ => {}
        }
    }
}

impl Drop for UsageTap {
    fn drop(&mut self) {
        if !self.sse && self.encoded.is_none() && !self.buf.is_empty() {
            if let Ok(v) = serde_json::from_slice::<Value>(&self.buf) {
                if v["type"] == "error" {
                    self.error = Some(v["error"].clone());
                } else {
                    self.message_id = v["id"].as_str().map(str::to_string);
                    self.model = v["model"].as_str().map(str::to_string);
                    self.stop_reason = v["stop_reason"].as_str().map(str::to_string);
                    self.take_usage(v.get("usage"));
                    for b in v["content"].as_array().into_iter().flatten() {
                        self.take_block(b);
                    }
                }
                self.stopped = true;
            }
        }
        let end_ms = log::now_ms();
        if self.status.is_some_and(|s| s >= 400) || self.error.is_some() {
            // Failures of routed requests would otherwise only show up in the log file.
            let model = self.record.get("model").and_then(Value::as_str).unwrap_or("?");
            let msg = self.error.as_ref().and_then(|e| e["message"].as_str()).unwrap_or("");
            eprintln!("[warn] {} {model} {}: {msg}", self.class, self.status.map_or("no response".into(), |s| s.to_string()));
        }
        self.turns.finish_request(ReqEnd {
            session: &self.session,
            prompt_id: self.prompt_id.as_deref(),
            class: &self.class,
            model: self.model.as_deref().or_else(|| self.record.get("model").and_then(Value::as_str)).unwrap_or(""),
            usage: &self.usage,
            tools: &self.tools_called,
            failed: self.error.is_some() || self.status.is_none_or(|s| s >= 400),
            stop_reason: self.stop_reason.as_deref(),
            complete: self.stopped,
            end_ms,
        });
        let mut r = std::mem::take(&mut self.record);
        r.insert("status".into(), json!(self.status));
        r.insert("request_id".into(), json!(self.request_id));
        r.insert("message_id".into(), json!(self.message_id));
        r.insert("response_model".into(), json!(self.model));
        r.insert("stop_reason".into(), json!(self.stop_reason));
        r.insert("usage".into(), Value::Object(std::mem::take(&mut self.usage)));
        r.insert("blocks".into(), json!(self.blocks));
        r.insert("tools_called".into(), json!(self.tools_called));
        r.insert("ratelimit".into(), Value::Object(std::mem::take(&mut self.ratelimit)));
        r.insert("content_encoding".into(), json!(self.encoded));
        r.insert("ttfb_ms".into(), json!(self.ttfb_ms));
        r.insert("latency_ms".into(), json!(self.started.elapsed().as_millis() as u64));
        r.insert("resp_bytes".into(), json!(self.bytes));
        r.insert("complete".into(), json!(self.stopped));
        r.insert("error".into(), self.error.take().unwrap_or(Value::Null));
        log::append(&self.path, &Value::Object(r));
    }
}

// --- per-turn rollup and outcome signals ---------------------------------------

/// What the gateway learns when a request starts a new user turn.
pub struct NewTurn {
    /// The user's previous prompt in this session, if any.
    pub prev_prompt: Option<String>,
}

/// How a request ended, for the turn rollup.
pub struct ReqEnd<'a> {
    pub session: &'a str,
    pub prompt_id: Option<&'a str>,
    pub class: &'a str,
    pub model: &'a str,
    pub usage: &'a Map<String, Value>,
    pub tools: &'a [String],
    pub failed: bool,
    pub stop_reason: Option<&'a str>,
    pub complete: bool,
    pub end_ms: u64,
}

#[derive(Default)]
struct SessionInfo {
    prompt_id: Option<String>,
    prompt: String,
    requested_model: String,
    turn_index: u32,
    last_end_ms: Option<u64>,
    cwd: Option<String>,
}

struct TurnAgg {
    ts_ms: u64,
    session: String,
    prompt_id: String,
    prompt_chars: usize,
    turn_index: u32,
    cwd: Option<String>,
    requested_model: String,
    decision: Value,
    followup: Map<String, Value>,
    requests: u32,
    by_class: BTreeMap<String, u32>,
    models: BTreeMap<String, u32>,
    tokens: BTreeMap<&'static str, u64>,
    tools: BTreeMap<String, u32>,
    errors: u32,
    aborted: bool,
    last_end_ms: u64,
    /// Background outcome checks still running; the turn is written once they land.
    pending: u32,
    /// Set when the turn finished while checks were pending: how it ended.
    closing: Option<String>,
}

#[derive(Default)]
struct TurnsInner {
    sessions: HashMap<String, SessionInfo>,
    open: HashMap<(String, String), TurnAgg>,
}

pub struct Turns {
    path: Option<String>,
    inner: Mutex<TurnsInner>,
}

impl Turns {
    pub fn new(path: Option<String>) -> Arc<Self> {
        Arc::new(Self { path, inner: Mutex::new(TurnsInner::default()) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TurnsInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cwd(&self, session: &str) -> Option<String> {
        self.lock().sessions.get(session).and_then(|s| s.cwd.clone())
    }

    /// Note a request. If it starts a new user turn (a `main` request whose prompt id
    /// differs from the session's last), closes the session's earlier turns, opens
    /// this one, and records how the user followed up on the previous turn.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &self,
        session: &str,
        prompt_id: Option<&str>,
        class: &str,
        req: &Value,
        requested_model: &str,
        prompt: Option<&str>,
        now_ms: u64,
    ) -> Option<NewTurn> {
        let mut g = self.lock();
        if g.sessions.get(session).is_none_or(|s| s.cwd.is_none()) {
            if let Some(c) = find_cwd(req) {
                g.sessions.entry(session.to_string()).or_default().cwd = Some(c.clone());
                // Turns opened before the cwd was known get it too.
                for ((s, _), a) in g.open.iter_mut() {
                    if s == session && a.cwd.is_none() {
                        a.cwd = Some(c.clone());
                    }
                }
            }
        }
        let info = g.sessions.entry(session.to_string()).or_default();
        let pid = prompt_id?;
        if class != "main" || info.prompt_id.as_deref() == Some(pid) {
            return None;
        }
        let prompt = prompt.unwrap_or("");
        let prev = info.prompt_id.is_some().then(|| info.prompt.clone());
        let mut followup = Map::new();
        if let Some(prev_prompt) = &prev {
            if let Some(end) = info.last_end_ms {
                followup.insert("gap_ms".into(), json!(now_ms.saturating_sub(end)));
            }
            followup.insert("similarity".into(), json!(similarity(prompt, prev_prompt)));
            followup.insert("model_switched".into(), json!(info.requested_model != requested_model));
            followup.insert("prev_requested_model".into(), json!(info.requested_model));
        }
        let turn_index = if prev.is_some() { info.turn_index + 1 } else { 0 };
        let cwd = info.cwd.clone();
        info.prompt_id = Some(pid.to_string());
        info.prompt = prompt.to_string();
        info.requested_model = requested_model.to_string();
        info.turn_index = turn_index;
        let agg = TurnAgg {
            ts_ms: now_ms,
            session: session.to_string(),
            prompt_id: pid.to_string(),
            prompt_chars: prompt.chars().count(),
            turn_index,
            cwd,
            requested_model: requested_model.to_string(),
            decision: Value::Null,
            followup,
            requests: 0,
            by_class: BTreeMap::new(),
            models: BTreeMap::new(),
            tokens: BTreeMap::new(),
            tools: BTreeMap::new(),
            errors: 0,
            aborted: false,
            last_end_ms: now_ms,
            pending: 0,
            closing: None,
        };
        g.open.insert((session.to_string(), pid.to_string()), agg);
        // Anything else still open for this session ended without a clean finish.
        let stale: Vec<_> = g.open.keys().filter(|(s, p)| s == session && p != pid).cloned().collect();
        for k in stale {
            if let Some(a) = g.open.remove(&k) {
                let ended = a.closing.clone();
                self.write_turn(a, ended);
            }
        }
        Some(NewTurn { prev_prompt: prev })
    }

    /// Attach the routing decision to the open turn.
    pub fn set_decision(&self, session: &str, prompt_id: &str, decision: Value) {
        if let Some(a) = self.lock().open.get_mut(&(session.to_string(), prompt_id.to_string())) {
            a.decision = decision;
        }
    }

    /// Register a background outcome check for the turn; the turn row waits for it.
    pub fn begin_check(&self, session: &str, prompt_id: &str) {
        if let Some(a) = self.lock().open.get_mut(&(session.to_string(), prompt_id.to_string())) {
            a.pending += 1;
        }
    }

    /// A check finished: record its signal (e.g. Jev's `correction_p`; `None` if it
    /// failed) and write the turn if it was only waiting for this.
    pub fn finish_check(&self, session: &str, prompt_id: &str, key: &str, value: Option<Value>) {
        let mut g = self.lock();
        let k = (session.to_string(), prompt_id.to_string());
        let Some(a) = g.open.get_mut(&k) else { return };
        if let Some(v) = value {
            a.followup.insert(key.into(), v);
        }
        a.pending = a.pending.saturating_sub(1);
        if a.pending == 0 && a.closing.is_some() {
            if let Some(a) = g.open.remove(&k) {
                let ended = a.closing.clone();
                self.write_turn(a, ended);
            }
        }
    }

    /// Fold a finished request into its turn, and write the turn when `main` ends
    /// cleanly with something other than a tool call.
    pub fn finish_request(&self, r: ReqEnd) {
        let Some(pid) = r.prompt_id else { return };
        let mut g = self.lock();
        let key = (r.session.to_string(), pid.to_string());
        let Some(a) = g.open.get_mut(&key) else { return };
        a.requests += 1;
        *a.by_class.entry(r.class.to_string()).or_default() += 1;
        *a.models.entry(r.model.to_string()).or_default() += 1;
        for f in TOKEN_FIELDS {
            if let Some(n) = r.usage.get(f).and_then(Value::as_u64) {
                *a.tokens.entry(f).or_default() += n;
            }
        }
        for t in r.tools {
            *a.tools.entry(t.clone()).or_default() += 1;
        }
        a.errors += r.failed as u32;
        a.last_end_ms = r.end_ms;
        if r.class != "main" {
            return;
        }
        if !r.complete {
            a.aborted = true;
        }
        let done = r.complete && !r.failed && !matches!(r.stop_reason, Some("tool_use" | "pause_turn"));
        let end = r.end_ms;
        if let Some(s) = g.sessions.get_mut(r.session) {
            s.last_end_ms = Some(end);
        }
        if done {
            let ended = r.stop_reason.unwrap_or("end_turn").to_string();
            if g.open.get(&key).is_some_and(|a| a.pending > 0) {
                g.open.get_mut(&key).expect("checked").closing = Some(ended);
            } else if let Some(a) = g.open.remove(&key) {
                self.write_turn(a, Some(ended));
            }
        }
    }

    fn write_turn(&self, a: TurnAgg, ended: Option<String>) {
        let Some(path) = &self.path else { return };
        let ended = ended.unwrap_or_else(|| {
            match (a.aborted, a.errors > 0) {
                (true, _) => "aborted",
                (_, true) => "error",
                _ => "superseded",
            }
            .into()
        });
        log::append(
            path,
            &json!({
                "ts_ms": a.ts_ms, "end_ms": a.last_end_ms, "wall_ms": a.last_end_ms.saturating_sub(a.ts_ms),
                "session": a.session, "prompt_id": a.prompt_id, "turn_index": a.turn_index,
                "cwd": a.cwd, "project": a.cwd.as_deref().map(project_of),
                "requested_model": a.requested_model, "prompt_chars": a.prompt_chars,
                "decision": a.decision, "followup": a.followup,
                "requests": a.requests, "by_class": a.by_class, "models": a.models,
                "tokens": a.tokens, "tools": a.tools, "errors": a.errors, "ended": ended,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap() -> UsageTap {
        UsageTap::new("/dev/null".into(), Instant::now(), json!({}), Turns::new(None), "s".into(), None, "main".into())
    }

    #[test]
    fn sse_lines_split_across_chunks_still_parse() {
        let mut t = tap();
        t.sse = true;
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"model\":\"x\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"name\":\"Read\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        for chunk in stream.as_bytes().chunks(7) {
            t.feed(chunk);
        }
        assert_eq!(t.usage["input_tokens"], 5);
        assert_eq!(t.usage["output_tokens"], 9);
        assert_eq!(t.tools_called, ["Read"]);
        assert_eq!(t.stop_reason.as_deref(), Some("tool_use"));
        assert!(t.stopped);
    }

    #[test]
    fn compressed_bodies_are_marked_not_parsed() {
        let mut t = tap();
        t.encoded = Some("gzip".into());
        t.feed(b"\x1f\x8b garbage");
        assert!(t.usage.is_empty() && t.buf.is_empty());
    }

    #[test]
    fn finds_cwd_and_project() {
        let req = json!({"messages": [{"role": "user", "content": [{"type": "text",
            "text": "x\n - Primary working directory: /home/u/Playground/app\n - Is a git repository: true"}]}]});
        let cwd = find_cwd(&req).unwrap();
        assert_eq!(cwd, "/home/u/Playground/app");
        assert_eq!(project_of(&cwd), "app");
        assert_eq!(find_cwd(&json!({"messages": []})), None);
    }

    #[test]
    fn cwd_found_in_system_prompt_or_late_message_but_not_in_tool_results() {
        let line = "Primary working directory: /home/u/app";
        let in_system = json!({"system": [{"type": "text", "text": format!("# Environment\n - {line}")}], "messages": []});
        assert_eq!(find_cwd(&in_system).as_deref(), Some("/home/u/app"));
        let msg = |c: Value| json!({"role": "user", "content": c});
        let filler = || msg(json!("hi"));
        let late = json!({"messages": [filler(), filler(), filler(), filler(), msg(json!([{"type": "text", "text": line}]))]});
        assert_eq!(find_cwd(&late).as_deref(), Some("/home/u/app"));
        let quoted = json!({"messages": [msg(json!([{"type": "tool_result", "content": line}]))]});
        assert_eq!(find_cwd(&quoted), None);
    }

    #[test]
    fn cwd_learned_mid_turn_reaches_the_open_turn() {
        let turns = Turns::new(None);
        let usage = Map::new();
        turns.observe("s", Some("p1"), "main", &json!({"messages": []}), "m", Some("hi"), 1);
        let with_cwd = json!({"system": "Primary working directory: /home/u/app", "messages": []});
        turns.observe("s", Some("p1"), "subagent", &with_cwd, "m", None, 2);
        assert_eq!(turns.lock().open.values().next().unwrap().cwd.as_deref(), Some("/home/u/app"));
        turns.finish_request(end("p1", &usage, "end_turn"));
    }

    #[test]
    fn request_params_drop_content_and_cap_big_values() {
        let req = json!({"model": "m", "messages": [1], "system": "s", "tools": [], "stream": true,
            "max_tokens": 10, "big": "x".repeat(2000)});
        let p = request_params(&req);
        assert_eq!(p["max_tokens"], 10);
        assert!(p.get("messages").is_none() && p.get("model").is_none());
        assert!(p["big"].as_str().unwrap().starts_with("<20"));
    }

    #[test]
    fn rephrase_scores_high_new_task_low() {
        assert!(similarity("fix the upload hang", "fix upload hang please") > 0.5);
        assert!(similarity("fix the upload hang", "add a unit test") < 0.2);
        assert_eq!(similarity("", ""), 0.0);
    }

    fn end<'a>(pid: &'a str, usage: &'a Map<String, Value>, stop: &'a str) -> ReqEnd<'a> {
        ReqEnd {
            session: "s", prompt_id: Some(pid), class: "main", model: "m", usage, tools: &[],
            failed: false, stop_reason: Some(stop), complete: true, end_ms: 10,
        }
    }

    #[test]
    fn turn_stays_open_across_tool_calls_and_closes_on_end_turn() {
        let turns = Turns::new(None);
        let req = json!({"messages": []});
        let usage = Map::new();
        assert!(turns.observe("s", Some("p1"), "main", &req, "m", Some("hi"), 1).is_some());
        // Tool-result requests of the same turn are not a new turn.
        assert!(turns.observe("s", Some("p1"), "main", &req, "m", None, 2).is_none());
        turns.finish_request(end("p1", &usage, "tool_use"));
        assert_eq!(turns.lock().open.len(), 1);
        turns.finish_request(end("p1", &usage, "end_turn"));
        assert!(turns.lock().open.is_empty());
    }

    #[test]
    fn closed_turn_waits_for_pending_check_and_new_turn_sweeps_the_old() {
        let turns = Turns::new(None);
        let req = json!({"messages": []});
        let usage = Map::new();
        turns.observe("s", Some("p1"), "main", &req, "m", Some("hi"), 1);
        turns.observe("s", Some("p2"), "main", &req, "m", Some("again"), 2);
        assert_eq!(turns.lock().open.len(), 1, "p1 swept when p2 began");
        turns.begin_check("s", "p2");
        turns.finish_request(end("p2", &usage, "end_turn"));
        assert_eq!(turns.lock().open.len(), 1, "held for the pending check");
        turns.finish_check("s", "p2", "correction_p", Some(json!(0.9)));
        assert!(turns.lock().open.is_empty());
    }
}
