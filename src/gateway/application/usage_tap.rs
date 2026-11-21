//! Usage logging for jev-gateway, as JSON lines for later analysis.
//!
//! - [`UsageTap`] watches one /v1/messages response as it streams to the client
//!   (it only reads; the client gets the bytes unchanged) and writes a request
//!   record: tokens, latency, status, tools the model called, rate-limit headers.
//! - [`Turns`] rolls requests up into one record per user turn and records how the
//!   user reacted to the previous turn (gap, rephrase, model switch), the outcome
//!   signal for judging routing later.


use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Map, Value};

use super::turns::{ReqEnd, Turns};
use crate::shared::application::host;

/// Cap on the text a tap keeps while parsing; usage fields are tiny, content is not.
const TAP_BUF_MAX: usize = 8 * 1024 * 1024;
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
        let end_ms = host::now_ms();
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
        host::append_line(&self.path, &Value::Object(r).to_string());
    }
}

// --- per-turn rollup and outcome signals ---------------------------------------

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
}
