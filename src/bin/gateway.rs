//! jev-gateway: a local Anthropic-format gateway that adds a "jev-code" model
//! to the normal Claude Code interface.
//!
//! Claude Code points `ANTHROPIC_BASE_URL` here. Every request is forwarded to
//! Anthropic unchanged (headers, body, streamed response), with one exception:
//! when the request's `model` is `jev-code`, the gateway swaps in a real model.
//! It decides once per user prompt (Claude Code's `x-claude-code-prompt-id` hint
//! header groups a turn's requests), using code mode's policy: Sonnet by default,
//! Opus for confident reasoning, strong security terms, or edits to auth code,
//! with the same upgrade-now / downgrade-slowly stickiness. Small background
//! requests (titles, summaries) go to Haiku, adapted to what Haiku accepts; forks of
//! the conversation and compaction stay on the turn's model; subagents run on Sonnet. Three JSON-lines logs: routing decisions, one usage
//! record per /v1/messages request, and one record per user turn (see README).
//!
//!   jev-gateway        (normally started by the `jev-claude` launcher)
//!
//! Env: JEV_GATEWAY_ADDR (127.0.0.1:18780), JEV_GATEWAY_UPSTREAM
//! (https://api.anthropic.com), CODE_STRONG_THRESHOLD (0.85), DOWNGRADE_AFTER (2),
//! JEV_GATEWAY_LOG (~/.local/state/jev-router/gateway.jsonl), JEV_USAGE_LOG
//! (usage.jsonl), JEV_TURNS_LOG (turns.jsonl).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, Method, Response, StatusCode};
use axum::routing::{any, get};
use axum::Router as Http;
use futures_util::StreamExt as _;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::log;
use jev_router::router::{Router, Sticky, CODE_STRONG_THRESHOLD};
use jev_router::typesafe::TypeSafe;
use jev_router::usage::{find_cwd, project_of, request_params, Turns, UsageTap};

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
    /// Routing record of the current turn, attached to the usage rows of its requests.
    decision: Option<Value>,
}

struct App {
    router: Router,
    http: reqwest::Client,
    upstream: String,
    downgrade_after: u32,
    sessions: Mutex<HashMap<String, SessionState>>,
    log_path: Option<String>,
    usage_path: Option<String>,
    turns: Arc<Turns>,
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
    let log_path = log::path_from_env("JEV_GATEWAY_LOG", "gateway.jsonl");
    let usage_path = log::path_from_env("JEV_USAGE_LOG", "usage.jsonl");
    let turns = Turns::new(log::path_from_env("JEV_TURNS_LOG", "turns.jsonl"));
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
        usage_path,
        turns,
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

async fn forward(app: &Arc<App>, req: Request) -> Result<Response<Body>> {
    let (parts, body) = req.into_parts();
    let path_q = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let mut bytes = to_bytes(body, MAX_BODY).await.context("read request body")?.to_vec();
    let started = Instant::now();
    let ts_ms = log::now_ms();

    // Only real message requests are logged; token counting and the rest pass through.
    let mut tap = None;
    let mut for_haiku = false;
    // The request as sent by the client, on the main model: retried if Haiku rejects it.
    let mut fallback: Option<Vec<u8>> = None;
    if parts.method == Method::POST && parts.uri.path() == "/v1/messages" {
        let meta = Meta::from_headers(&parts.headers);
        let json: Option<Value> = serde_json::from_slice(&bytes).ok();
        let requested = json.as_ref().and_then(|j| j.get("model")).and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(j) = &json {
            let prompt = last_user_prompt(j);
            let new_turn = app.turns.observe(&meta.session, meta.prompt_id.as_deref(), &meta.class, j, &requested, prompt.as_deref(), ts_ms);
            if let (true, Some(prev), Some(pid), Some(prompt)) = (correction_check_enabled(), new_turn.and_then(|n| n.prev_prompt), &meta.prompt_id, prompt) {
                app.turns.begin_check(&meta.session, pid);
                spawn_outcome_check(app, meta.session.clone(), pid.clone(), prompt, prev, recent_context(j));
            }
        }
        let picked = match &json {
            Some(j) => choose_model(app, &meta, j).await,
            None => None,
        };
        if let (Some(p), Some(pid)) = (&picked, &meta.prompt_id) {
            if let (true, Some(d)) = (p.new_turn, &p.decision) {
                app.turns.set_decision(&meta.session, pid, d.clone());
            }
        }
        let model = picked.as_ref().map(|p| p.model.to_string()).unwrap_or_else(|| requested.clone());
        if let Some(p) = &picked {
            if p.model == MODEL_CHEAP {
                fallback = Some(set_model(&bytes, MODEL_MID));
            }
            bytes = set_model(&bytes, p.model);
            if let (MODEL_CHEAP, Some(j)) = (p.model, &json) {
                let mut adapted = j.clone();
                adapted["model"] = Value::String(MODEL_CHEAP.into());
                adapt_for_haiku(&mut adapted);
                bytes = serde_json::to_vec(&adapted).unwrap_or(bytes);
                for_haiku = true;
            }
        }
        tap = app.usage_path.clone().map(|path| {
            let j = json.as_ref();
            let cwd = app.turns.cwd(&meta.session).or_else(|| j.and_then(find_cwd));
            UsageTap::new(
                path,
                started,
                json!({
                    "ts_ms": ts_ms, "session": meta.session, "prompt_id": meta.prompt_id, "class": meta.class,
                    "cwd": cwd, "project": cwd.as_deref().map(project_of),
                    "requested_model": requested, "model": model, "routed": picked.is_some(),
                    "new_turn": picked.as_ref().is_some_and(|p| p.new_turn), "adapted": for_haiku,
                    "decision": picked.as_ref().and_then(|p| p.decision.clone()),
                    "stream": j.and_then(|j| j.get("stream")).and_then(Value::as_bool).unwrap_or(false),
                    "params": j.map(request_params),
                    "beta": header(&parts.headers, "anthropic-beta"),
                    "client": header(&parts.headers, "user-agent"),
                    "messages": j.and_then(|j| j.get("messages")).and_then(Value::as_array).map(Vec::len),
                    "tools": j.and_then(|j| j.get("tools")).and_then(Value::as_array).map(Vec::len),
                    "req_bytes": bytes.len(),
                }),
                app.turns.clone(),
                meta.session.clone(),
                meta.prompt_id.clone(),
                meta.class.clone(),
            )
        });
    }

    let logged = tap.is_some();
    let sent = upstream_request(app, &parts, &path_q, bytes, for_haiku, logged).send().await;
    let mut resp = match sent {
        Ok(r) => r,
        Err(e) => {
            if let Some(t) = &mut tap {
                t.error = Some(json!({ "type": "gateway_upstream", "message": format!("{e:#}") }));
            }
            return Err(anyhow::Error::new(e).context("upstream request failed"));
        }
    };
    // Haiku has restrictions that can change without notice. Rather than lose a
    // background request, retry it once as sent by the client, on the main model.
    if let (Some(retry), true) = (fallback, matches!(resp.status().as_u16(), 400 | 413 | 422)) {
        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap_or_default();
        let msg = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_default();
        eprintln!("[warn] {MODEL_CHEAP} rejected a request ({status}): {msg} — retrying on {MODEL_MID}");
        if let Some(t) = &mut tap {
            t.set("fallback_from", json!({ "model": MODEL_CHEAP, "status": status, "error": msg }));
            t.set("model", json!(MODEL_MID));
            t.set("adapted", json!(false));
        }
        resp = match upstream_request(app, &parts, &path_q, retry, false, logged).send().await {
            Ok(r) => r,
            Err(e) => {
                if let Some(t) = &mut tap {
                    t.error = Some(json!({ "type": "gateway_upstream", "message": format!("{e:#}") }));
                }
                return Err(anyhow::Error::new(e).context("upstream request failed"));
            }
        };
    }

    let mut out = Response::builder().status(resp.status().as_u16());
    for (k, v) in resp.headers().iter() {
        if !SKIP_HEADERS.contains(&k.as_str()) {
            out = out.header(HeaderName::from_bytes(k.as_str().as_bytes())?, v.as_bytes());
        }
    }
    // Stream the response through as it arrives (SSE pings included). The tap only
    // reads the chunks; what the client receives is unchanged.
    if let Some(t) = &mut tap {
        t.start_response(resp.status().as_u16(), resp.headers());
    }
    let stream = resp.bytes_stream();
    let body = match tap {
        Some(mut t) => {
            Body::from_stream(stream.map(move |chunk| {
                match &chunk {
                    Ok(c) => t.feed(c),
                    Err(e) => t.error = Some(json!({ "type": "gateway_stream", "message": e.to_string() })),
                }
                chunk
            }))
        }
        None => Body::from_stream(stream),
    };
    Ok(out.body(body)?)
}

/// The upstream request: the client's headers, minus hop-by-hop ones. A logged request
/// asks for an uncompressed response so the tap can read it; a Haiku request loses the
/// long-context beta.
fn upstream_request(app: &App, parts: &axum::http::request::Parts, path_q: &str, body: Vec<u8>, for_haiku: bool, logged: bool) -> reqwest::RequestBuilder {
    let mut up = app.http.request(parts.method.clone(), format!("{}{}", app.upstream, path_q));
    for (k, v) in parts.headers.iter() {
        if SKIP_HEADERS.contains(&k.as_str()) || (logged && k == "accept-encoding") {
            continue;
        }
        match (for_haiku && k == "anthropic-beta", v.to_str()) {
            (true, Ok(list)) => {
                let list = without_long_context(list);
                if !list.is_empty() {
                    up = up.header(k, list);
                }
            }
            _ => up = up.header(k, v),
        }
    }
    up.body(body)
}

/// The LLM-judged `correction_p` is opt-in (`JEV_CORRECTION_CHECK=1`): on 276 logged
/// turns it did not correlate with aborts, quick follow-ups or model switches, so it
/// is not worth a classifier call per turn until it is validated against real labels.
fn correction_check_enabled() -> bool {
    std::env::var("JEV_CORRECTION_CHECK").is_ok_and(|v| v == "1")
}

/// Background: ask Jev whether the user's new prompt reacts to the last answer as
/// wrong or incomplete. The result lands in the turn's `followup.correction_p`; the
/// turn row waits for it. Never delays or affects routing.
fn spawn_outcome_check(app: &Arc<App>, session: String, prompt_id: String, prompt: String, prev_prompt: String, prev_answer: String) {
    let app = app.clone();
    tokio::spawn(async move {
        let check = app.router.is_correction(&prompt, &prev_prompt, &prev_answer);
        let p = tokio::time::timeout(std::time::Duration::from_secs(20), check).await.ok().and_then(Result::ok);
        app.turns.finish_check(&session, &prompt_id, "correction_p", p.map(|p| json!(p)));
    });
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// Claude Code's per-request hint headers.
struct Meta {
    class: String,
    session: String,
    prompt_id: Option<String>,
}

impl Meta {
    fn from_headers(h: &HeaderMap) -> Self {
        Self {
            class: header(h, "x-claude-code-request-class").unwrap_or("main").to_string(),
            session: header(h, "x-claude-code-session-id").unwrap_or("default").to_string(),
            prompt_id: header(h, "x-claude-code-prompt-id").map(str::to_string),
        }
    }
}

/// A routed request: the real model, and the turn's decision for the usage log.
struct Picked {
    model: &'static str,
    new_turn: bool,
    decision: Option<Value>,
}

/// The real model for a `jev-code` request, or None to forward it untouched.
async fn choose_model(app: &App, meta: &Meta, req: &Value) -> Option<Picked> {
    if req.get("model")?.as_str()? != ALIAS {
        return None;
    }
    let (class, session_id, prompt_id) = (meta.class.as_str(), meta.session.clone(), meta.prompt_id.clone());
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(session_id.clone()).or_default();
    let same_turn = |st: &SessionState| Picked {
        model: st.model.unwrap_or(MODEL_MID),
        new_turn: false,
        decision: st.decision.clone(),
    };

    if class == "auxiliary" {
        // A fork of the conversation (carries the tool list) shares the main model's
        // prompt cache and can exceed Haiku's 200k window; only small one-shot
        // requests (titles, summaries) go to Haiku.
        let fork = req.get("tools").and_then(Value::as_array).is_some_and(|t| !t.is_empty());
        return Some(if fork { same_turn(st) } else { Picked { model: MODEL_CHEAP, new_turn: false, decision: None } });
    }
    if class == "subagent" {
        // Subagents arriving as jev-code (the main model named no model) are mostly
        // searches and reads. Sonnet for every request keeps a run on one model, so a
        // main-model switch can't rebuild the subagent's cache mid-run. An explicit
        // model on the Agent tool never reaches here: only the alias is routed.
        return Some(Picked { model: MODEL_MID, new_turn: false, decision: None });
    }
    if class != "main" {
        // Workflows, compaction: stay on the turn's model.
        return Some(same_turn(st));
    }

    let prompt = last_user_prompt(req);
    let new_turn = match (&prompt_id, &st.prompt_id) {
        (Some(p), Some(prev)) => p != prev,
        (Some(_), None) => true,
        // No hint headers: a user message with real text (not just tool results) starts a turn.
        (None, _) => prompt.is_some(),
    };
    if !new_turn || prompt.is_none() {
        return Some(same_turn(st));
    }
    let prompt = prompt.unwrap();
    // Not something the user typed: don't route it or touch the sticky counters.
    if is_harness_message(&prompt) {
        return Some(same_turn(st));
    }
    let context = recent_context(req);
    let fallback = st.sticky.tier.unwrap_or(1);
    drop(sessions); // don't hold the lock across the Jev call

    let want = app.router.code_want(&prompt, &context, fallback).await;
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(session_id.clone()).or_default();
    let (tier, reason) = st.sticky.decide(&want, app.downgrade_after);
    st.sticky.tier = Some(tier);
    st.prompt_id = prompt_id.clone();
    st.model = Some(TIERS[tier]);
    let decision = json!({ "tier": tier, "reason": reason, "confident": want.confident, "signals": want.signals });
    st.decision = Some(decision.clone());
    log_decision(app, &session_id, prompt_id.as_deref(), TIERS[tier], &decision, &prompt);
    Some(Picked { model: TIERS[tier], new_turn: true, decision: Some(decision) })
}

/// Messages Claude Code puts in the user slot that the user never typed: a finished
/// subagent's report, and the notice that a permission was granted. Routing them as
/// prompts sent every hand-back to Opus on the security keyword rule (169 of 169 in
/// the logs) and reset the step-down counters, so a session that delegated never left
/// Opus. They continue the current turn on whatever model it is already on.
fn is_harness_message(prompt: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "Another Claude session sent a message:",
        "Permission granted for this exact command only",
    ];
    let p = prompt.trim_start();
    PREFIXES.iter().any(|x| p.starts_with(x))
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

/// A 1M-context session makes Claude Code add settings Haiku 4.5 rejects: the
/// long-context beta, adaptive thinking, effort, and mid-conversation system
/// messages. Claude Code only finds out by failing, so adapt the request up front.
fn adapt_for_haiku(req: &mut Value) {
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

fn without_long_context(beta: &str) -> String {
    beta.split(',').map(str::trim).filter(|b| !b.is_empty() && !b.starts_with("context-1m")).collect::<Vec<_>>().join(",")
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

fn log_decision(app: &App, session: &str, prompt_id: Option<&str>, model: &str, decision: &Value, prompt: &str) {
    eprintln!("[route] {model} — {}", decision["reason"].as_str().unwrap_or(""));
    let Some(path) = &app.log_path else { return };
    log::append(
        path,
        &json!({
            "ts": log::now_ms() / 1000, "session": session, "prompt_id": prompt_id, "model": model,
            "reason": decision["reason"], "signals": decision["signals"],
            "prompt": prompt.chars().take(200).collect::<String>(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn real_prompts_are_still_routed() {
        for p in [
            "check the upload route for path traversal",
            "another claude session sent a message, why?",
            "is the permission granted for this command in settings.json?",
            "thanks!",
        ] {
            assert!(!is_harness_message(p), "{p}");
        }
    }

    #[test]
    fn long_context_beta_is_stripped_from_the_list() {
        assert_eq!(
            without_long_context("claude-code-20250219,oauth-2025-04-20,context-1m-2025-08-07,interleaved-thinking-2025-05-14"),
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14"
        );
        assert_eq!(without_long_context("context-1m-2025-08-07"), "");
    }
}
