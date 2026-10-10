//! jev-gateway: a local Anthropic-format gateway that adds a "jev-code" model
//! to the normal Claude Code interface.
//!
//! Claude Code points `ANTHROPIC_BASE_URL` here. Every request is forwarded to
//! Anthropic unchanged (headers, body, streamed response), with one exception:
//! when the request's `model` is `jev-code`, the gateway swaps in a real model.
//! It decides once per user prompt (Claude Code's `x-claude-code-prompt-id` hint
//! header groups a turn's requests), using code mode's policy: Sonnet by default,
//! Opus for confident reasoning, strong security terms, or edits to auth code,
//! with the same upgrade-now / downgrade-slowly stickiness. Background requests
//! (titles, summaries) go to Haiku. Decisions are logged as JSON lines.
//!
//!   jev-gateway        (normally started by the `jev-claude` launcher)
//!
//! Env: JEV_GATEWAY_ADDR (127.0.0.1:18780), JEV_GATEWAY_UPSTREAM
//! (https://api.anthropic.com), CODE_STRONG_THRESHOLD (0.85), DOWNGRADE_AFTER (2),
//! JEV_GATEWAY_LOG (~/.local/state/jev-router/gateway.jsonl).

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, Method, Response, StatusCode};
use axum::routing::{any, get};
use axum::Router as Http;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::{Router, Sticky, CODE_STRONG_THRESHOLD};
use jev_router::typesafe::TypeSafe;

const ALIAS: &str = "jev-code";
const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Hop-by-hop and recomputed headers that must not be copied across the proxy.
const SKIP_HEADERS: &[&str] =
    &["host", "content-length", "connection", "keep-alive", "transfer-encoding", "upgrade", "te", "trailer"];

#[derive(Default)]
struct SessionState {
    sticky: Sticky,
    prompt_id: Option<String>,
    model: Option<&'static str>,
}

struct App {
    router: Router,
    http: reqwest::Client,
    upstream: String,
    downgrade_after: u32,
    sessions: Mutex<HashMap<String, SessionState>>,
    log_path: Option<String>,
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(a) = std::env::args().nth(1) {
        match a.as_str() {
            "-V" | "--version" => {
                println!("jev-gateway {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                println!("jev-gateway: local gateway that adds the \"{ALIAS}\" model to Claude Code. Start it with jev-claude.");
                return Ok(());
            }
        }
    }
    let addr = std::env::var("JEV_GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:18780".into());
    let log_path = std::env::var("JEV_GATEWAY_LOG").ok().or_else(|| {
        let dir = format!("{}/.local/state/jev-router", std::env::var("HOME").ok()?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(format!("{dir}/gateway.jsonl"))
    });
    let app = Arc::new(App {
        router: Router::new(TypeSafe::from_env()?, 1.0).with_strong_threshold(env_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD)),
        http: reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()?,
        upstream: std::env::var("JEV_GATEWAY_UPSTREAM")
            .unwrap_or_else(|_| "https://api.anthropic.com".into())
            .trim_end_matches('/')
            .to_string(),
        downgrade_after: env_f64("DOWNGRADE_AFTER", 2.0) as u32,
        sessions: Mutex::new(HashMap::new()),
        log_path,
    });
    let http = Http::new()
        .route("/healthz", get(|| async { "ok" }))
        .fallback(any(proxy))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| format!("bind {addr}"))?;
    eprintln!("jev-gateway listening on http://{addr}");
    axum::serve(listener, http).await?;
    Ok(())
}

async fn proxy(State(app): State<Arc<App>>, req: Request) -> Response<Body> {
    match forward(&app, req).await {
        Ok(r) => r,
        Err(e) => {
            // Anthropic-shaped error so Claude Code shows and retries it normally.
            let body = json!({"type": "error", "error": {"type": "api_error", "message": format!("jev-gateway: {e:#}")}});
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        }
    }
}

async fn forward(app: &App, req: Request) -> Result<Response<Body>> {
    let (parts, body) = req.into_parts();
    let path_q = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let mut bytes = to_bytes(body, MAX_BODY).await.context("read request body")?.to_vec();

    if parts.method == Method::POST && parts.uri.path().starts_with("/v1/messages") {
        if let Some(model) = choose_model(app, &parts.headers, &bytes, parts.uri.path()).await {
            bytes = set_model(&bytes, model);
        }
    }

    let mut up = app.http.request(parts.method.clone(), format!("{}{}", app.upstream, path_q));
    for (k, v) in parts.headers.iter() {
        if !SKIP_HEADERS.contains(&k.as_str()) {
            up = up.header(k, v);
        }
    }
    let resp = up.body(bytes).send().await.context("upstream request failed")?;

    let mut out = Response::builder().status(resp.status().as_u16());
    for (k, v) in resp.headers().iter() {
        if !SKIP_HEADERS.contains(&k.as_str()) {
            out = out.header(HeaderName::from_bytes(k.as_str().as_bytes())?, v.as_bytes());
        }
    }
    // Stream the response through as it arrives (SSE pings included).
    Ok(out.body(Body::from_stream(resp.bytes_stream()))?)
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// The real model for a `jev-code` request, or None to forward it untouched.
async fn choose_model(app: &App, headers: &HeaderMap, body: &[u8], path: &str) -> Option<&'static str> {
    let req: Value = serde_json::from_slice(body).ok()?;
    if req.get("model")?.as_str()? != ALIAS {
        return None;
    }
    let class = header(headers, "x-claude-code-request-class").unwrap_or("main");
    let session_id = header(headers, "x-claude-code-session-id").unwrap_or("default").to_string();
    let prompt_id = header(headers, "x-claude-code-prompt-id").map(str::to_string);
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(session_id.clone()).or_default();

    // Token counting and side requests never trigger a routing decision.
    if path.ends_with("/count_tokens") {
        return Some(st.model.unwrap_or(MODEL_MID));
    }
    if class == "auxiliary" {
        return Some(MODEL_CHEAP);
    }
    if class != "main" {
        // Subagents, workflows, compaction: stay on the turn's model.
        return Some(st.model.unwrap_or(MODEL_MID));
    }

    let prompt = last_user_prompt(&req);
    let new_turn = match (&prompt_id, &st.prompt_id) {
        (Some(p), Some(prev)) => p != prev,
        (Some(_), None) => true,
        // No hint headers: a user message with real text (not just tool results) starts a turn.
        (None, _) => prompt.is_some(),
    };
    if !new_turn || prompt.is_none() {
        return Some(st.model.unwrap_or(MODEL_MID));
    }
    let prompt = prompt.unwrap();
    let context = recent_context(&req);
    let fallback = st.sticky.tier.unwrap_or(1);
    drop(sessions); // don't hold the lock across the Jev call

    let want = app.router.code_want(&prompt, &context, fallback).await;
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(session_id.clone()).or_default();
    let (tier, reason) = st.sticky.decide(&want, app.downgrade_after);
    st.sticky.tier = Some(tier);
    st.prompt_id = prompt_id.clone();
    st.model = Some(TIERS[tier]);
    log_decision(app, &session_id, prompt_id.as_deref(), TIERS[tier], &reason, &prompt);
    Some(TIERS[tier])
}

/// Text of the newest user message, ignoring tool results and Claude Code's
/// injected <system-reminder> blocks. None if it carries no real text.
fn last_user_prompt(req: &Value) -> Option<String> {
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

fn strip_reminders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<system-reminder>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</system-reminder>") {
            Some(end) => rest = &rest[start + end + "</system-reminder>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The previous assistant reply (trimmed), as context for Jev.
fn recent_context(req: &Value) -> String {
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

/// Replace the model value, leaving every other byte of the body as sent.
fn set_model(body: &[u8], model: &str) -> Vec<u8> {
    let needle = format!("\"model\":\"{ALIAS}\"");
    let text = String::from_utf8_lossy(body);
    if let Some(i) = text.find(&needle) {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..i]);
        out.push_str(&format!("\"model\":\"{model}\""));
        out.push_str(&text[i + needle.len()..]);
        return out.into_bytes();
    }
    // Unusual spacing: fall back to a JSON round-trip (same content, new whitespace).
    match serde_json::from_slice::<Value>(body) {
        Ok(mut v) => {
            v["model"] = Value::String(model.into());
            serde_json::to_vec(&v).unwrap_or_else(|_| body.to_vec())
        }
        Err(_) => body.to_vec(),
    }
}

fn log_decision(app: &App, session: &str, prompt_id: Option<&str>, model: &str, reason: &str, prompt: &str) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    eprintln!("[route] {model} — {reason}");
    let Some(path) = &app.log_path else { return };
    let line = json!({
        "ts": ts, "session": session, "prompt_id": prompt_id, "model": model, "reason": reason,
        "prompt": prompt.chars().take(200).collect::<String>(),
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
