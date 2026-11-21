//! Test rig for the golden tests: a fake Jev and a fake Anthropic on loopback ports, and
//! the gateway's own logs in a temporary directory. Values that differ per run are
//! replaced by `<key>` before comparing.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use jev_catalog::domain::model_table::MODEL_STRONG;
use crate::application::app_state::{App, Orch};
use crate::application::session_state::Sessions;
use jev_judgment::application::judge::Router;
use jev_judgment::domain::thresholds::{CODE_STRONG_THRESHOLD, TOP_THRESHOLD};
use jev_judgment::infrastructure::typesafe_http::TypeSafe;

use super::proxy_handler::{forward, MAX_BODY};

pub(super) type Seen = Arc<std::sync::Mutex<Vec<String>>>;

/// A loopback HTTP server. `reply(body)` makes (status, content type, body) for each
/// request; the bodies received are kept in order.
pub(super) async fn fake_server(reply: fn(&str) -> (u16, &'static str, String)) -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let body = loop {
                    let Ok(n) = sock.read(&mut chunk).await else { return };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                    let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        break String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).into_owned();
                    }
                };
                let (status, kind, out) = reply(&body);
                log.lock().unwrap().push(body);
                let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}", out.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (url, seen)
}

/// Jev answers every question asked: `frontend` for the choice, a fixed probability per noul.
pub(super) fn jev_answers(body: &str) -> (u16, &'static str, String) {
    let req: Value = serde_json::from_str(body).unwrap();
    let mut answers = serde_json::Map::new();
    for (name, q) in req["questions"].as_object().unwrap() {
        let a = if q["type"] == "choice" {
            json!({ "choice": "frontend", "confidence": 0.8, "probabilities": { "frontend": 0.8, "docs": 0.2 } })
        } else {
            let p = match name.as_str() {
                "continues" => 0.11,
                "needs_changes" => 0.62,
                "security_review" => 0.33,
                "very_hard" => 0.44,
                "multi_part" => 0.66,
                "sequential" => 0.11,
                _ => 0.99,
            };
            json!({ "noul": p })
        };
        answers.insert(name.clone(), a);
    }
    (200, "application/json", json!({ "answers": answers }).to_string())
}

pub(super) fn jev_down(_: &str) -> (u16, &'static str, String) {
    (500, "application/json", "{}".into())
}

/// Anthropic answers a streamed request with SSE and any other with one JSON message.
pub(super) fn anthropic_answers(body: &str) -> (u16, &'static str, String) {
    if body.contains("\"stream\":true") {
        let events = [
            r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-x","usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"Bash","input":{}}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        (200, "text/event-stream", events.iter().map(|e| format!("data: {e}\n\n")).collect())
    } else {
        let msg = r#"{"id":"msg_2","type":"message","model":"claude-haiku-x","stop_reason":"end_turn","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":3,"output_tokens":2}}"#;
        (200, "application/json", msg.into())
    }
}

pub(super) struct Rig {
    pub(super) app: Arc<App>,
    dir: std::path::PathBuf,
    pub(super) upstream: Seen,
}

impl Rig {
    pub(super) async fn new(name: &str, jev: fn(&str) -> (u16, &'static str, String)) -> Self {
        static RIGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = RIGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jev-golden-gateway-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
        let (jev_url, _) = fake_server(jev).await;
        let (upstream_url, upstream) = fake_server(anthropic_answers).await;
        let app = Arc::new(App {
            router: Router::new(TypeSafe::with_url("k", &format!("{jev_url}/v1/systemone")), 1.0)
                .with_strong_threshold(CODE_STRONG_THRESHOLD)
                .with_top_threshold(Some(TOP_THRESHOLD)),
            http: reqwest::Client::new(),
            upstream: upstream_url,
            downgrade_after: 2,
            sessions: Mutex::new(Sessions::new(Arc::new(jev_shared::infrastructure::system_host::SystemHost))),
            log_path: path("gateway.jsonl"),
            usage_path: path("usage.jsonl"),
            turns_path: path("turns.jsonl"),
            orch: Orch { enabled: true, cutoff: 0.5, planner: MODEL_STRONG, planner_pinned: false, require_opus: true, log_path: path("orch.jsonl") },
        });
        Self { app, dir, upstream }
    }

    /// One `/v1/messages` request through `forward`, the response read to its end.
    pub(super) async fn send(&self, session: &str, prompt_id: Option<&str>, class: &str, body: &str) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("x-api-key", "sk-test")
            .header("user-agent", "golden-client/1.0")
            .header("anthropic-beta", "context-1m-2025-08-07,fine-grained-tool-streaming-2025-05-14")
            .header("x-claude-code-session-id", session)
            .header("x-claude-code-request-class", class);
        if let Some(p) = prompt_id {
            req = req.header("x-claude-code-prompt-id", p);
        }
        let resp = forward(&self.app, req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        to_bytes(resp.into_body(), MAX_BODY).await.unwrap();
    }

    /// The lines of one of the gateway's logs, with the values that change per run masked.
    pub(super) fn log(&self, file: &str) -> String {
        let text = std::fs::read_to_string(self.dir.join(file)).unwrap_or_default();
        text.lines().map(|l| masked_line(l) + "\n").collect()
    }

    pub(super) fn bodies(&self) -> String {
        self.upstream.lock().unwrap().iter().map(|b| format!("{b}\n")).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(super) const GOLDEN_MASK: &str = "the values of ts, ts_ms, end_ms, wall_ms, latency_ms, ttfb_ms and gap_ms (each becomes <key>)";

pub(super) fn mask_value(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                if ["ts", "ts_ms", "end_ms", "wall_ms", "latency_ms", "ttfb_ms", "gap_ms"].contains(&k.as_str()) {
                    *x = Value::String(format!("<{k}>"));
                } else {
                    mask_value(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(mask_value),
        _ => {}
    }
}

pub(super) fn masked_line(line: &str) -> String {
    let mut v: Value = serde_json::from_str(line).unwrap();
    mask_value(&mut v);
    v.to_string()
}
