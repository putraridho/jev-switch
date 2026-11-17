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
//! (usage.jsonl), JEV_TURNS_LOG (turns.jsonl). WORKER_CHEAP_P (unset: workers never get Haiku), ORCH_REQUIRE_OPUS (on). Fable tier: JEV_FABLE (on; "off" disables),
//! FABLE_THRESHOLD (0.90). Orchestrate mode (README): JEV_ORCH
//! (on; "off" disables), ORCH_TRIGGER_P (0.70), ORCH_PLANNER (claude-opus-5-5),
//! JEV_ORCH_LOG (orch.jsonl).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::body::{to_bytes, Body};
use axum::extract::{Json, Request, State};
use axum::http::{HeaderMap, HeaderName, Method, Response, StatusCode};
use axum::routing::{any, get, post};
use axum::Router as Http;
use futures_util::StreamExt as _;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP};
use jev_router::log;
use jev_router::orchestrate::{orch_gate, Gate};
use jev_router::router::{Router, Sticky, CODE_STRONG_THRESHOLD, ORCH_TRIGGER_P, TOP_THRESHOLD};
use jev_router::typesafe::TypeSafe;
use jev_router::prefix::PrefixWatch;
use jev_router::usage::{find_cwd, project_of, request_params, Turns, UsageTap};

const ALIAS: &str = "jev-code";
/// Start of the context block the jev-orchestrate mod attaches to a prompt that
/// triggered orchestration. The gateway only reads it; it never edits the request.
const ORCH_MARKER: &str = "<jev-orchestrate>";
const TIERS: [&str; 4] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP];
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
    /// The latest orchestrate check for this session (`multi_part_p`), recorded on the
    /// next turn's decision whether or not it triggered, as a label for tuning.
    orch_check: Option<Value>,
    /// The planner model of the current turn when it is orchestrated, else `None`: a
    /// worker may be Opus only when this is Fable.
    orch_planner: Option<&'static str>,
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
    prefix: PrefixWatch,
    orch: Orch,
}

/// Orchestrate mode settings (see README "Orchestrate mode").
struct Orch {
    /// `JEV_ORCH=off` turns the trigger off for every session.
    enabled: bool,
    /// `ORCH_TRIGGER_P`: `multi_part` probability at or above which a prompt triggers.
    cutoff: f64,
    /// The model the main thread runs on for an orchestrated turn when Jev has no say:
    /// Opus, or `ORCH_PLANNER` if set.
    planner: &'static str,
    /// `ORCH_PLANNER` is set: it pins the planner and Jev does not pick it.
    planner_pinned: bool,
    /// `ORCH_REQUIRE_OPUS` (default on; `0` or `off` disables): orchestrate only a prompt
    /// that normal routing would have sent to Opus. When Sonnet alone would do the work,
    /// orchestration only adds an expensive planner (15x a Sonnet run on a small task).
    require_opus: bool,
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
    let log_path = log::path_from_env("JEV_GATEWAY_LOG", "gateway.jsonl");
    let usage_path = log::path_from_env("JEV_USAGE_LOG", "usage.jsonl");
    let turns = Turns::new(log::path_from_env("JEV_TURNS_LOG", "turns.jsonl"));
    let app = Arc::new(App {
        router: Router::new(TypeSafe::from_env()?, 1.0).with_strong_threshold(env_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD))
            // Fable for the hardest Opus-bound turns; JEV_FABLE=off keeps tiers 0-2.
            .with_worker_cheap(std::env::var("WORKER_CHEAP_P").ok().and_then(|v| v.parse().ok()))
            .with_top_threshold(std::env::var("JEV_FABLE").map_or(true, |v| v != "off").then(|| env_f64("FABLE_THRESHOLD", TOP_THRESHOLD))),
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
        prefix: PrefixWatch::new(),
        orch: Orch {
            enabled: std::env::var("JEV_ORCH").map_or(true, |v| v != "off"),
            cutoff: env_f64("ORCH_TRIGGER_P", ORCH_TRIGGER_P),
            // Leaked once at startup: routing hands out &'static model names.
            planner: Box::leak(std::env::var("ORCH_PLANNER").unwrap_or_else(|_| MODEL_STRONG.into()).into_boxed_str()),
            planner_pinned: std::env::var("ORCH_PLANNER").is_ok(),
            require_opus: std::env::var("ORCH_REQUIRE_OPUS").map_or(true, |v| v != "0" && v != "off"),
            log_path: log::path_from_env("JEV_ORCH_LOG", "orch.jsonl"),
        },
    });
    let http = Http::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/jev/orchestrate", post(orchestrate_check))
        .route("/jev/worker", post(worker_check))
        .fallback(any(proxy))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| format!("bind {addr}"))?;
    eprintln!("jev-gateway listening on http://{addr}");
    axum::serve(listener, http).await?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct OrchCheck {
    prompt: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    session: String,
}

/// `POST /jev/orchestrate`, called by the jev-orchestrate mod as a prompt is
/// submitted: should this prompt be orchestrated? Never fails the caller: if Jev is
/// down the answer is no.
async fn orchestrate_check(State(app): State<Arc<App>>, Json(c): Json<OrchCheck>) -> Json<Value> {
    if !app.orch.enabled {
        return Json(json!({ "trigger": false, "enabled": false }));
    }
    // Not typed by the user: no Jev call, no trigger (the mod skips these too).
    let words = if is_harness_message(&c.prompt) { None } else { user_words(&c.prompt) };
    let Some(words) = words else {
        return Json(json!({ "trigger": false, "enabled": true, "skipped": "not typed" }));
    };
    let check = async { app.router.orchestration_p(&words, &c.context).await.map(|(score, _, _)| score) };
    let p = match tokio::time::timeout(std::time::Duration::from_secs(10), check).await {
        Ok(Ok(p)) => Some(p),
        Ok(Err(e)) => {
            eprintln!("[warn] orchestrate check: jev failed: {e:#}");
            None
        }
        Err(_) => {
            eprintln!("[warn] orchestrate check: jev timed out");
            None
        }
    };
    // A prompt that clears the multi-part cutoff is also routed the normal way, which
    // tells us whether Opus would have run it anyway, and which planner it needs: Fable
    // when routing escalates it as very hard (and ORCH_PLANNER doesn't pin one).
    let mut route_tier = None;
    let mut hard_p = None;
    if p.is_some_and(|p| p >= app.orch.cutoff) {
        let want = app.router.code_want(&words, &c.context, 1).await;
        if !want.signals.is_null() {
            route_tier = Some(want.tier);
            hard_p = want.signals["hard_p"].as_f64();
        }
    }
    let gate = orch_gate(p, app.orch.cutoff, route_tier, app.orch.require_opus);
    let trigger = gate == Gate::Trigger;
    let planner = if trigger && !app.orch.planner_pinned && route_tier.is_some_and(|t| t >= 3) { MODEL_TOP } else { app.orch.planner };
    let skipped = (gate == Gate::SingleModelRoute).then_some("normal routing would not use Opus");
    let record = json!({
        "multi_part_p": p, "trigger": trigger, "cutoff": app.orch.cutoff, "planner": planner, "hard_p": hard_p,
        "route_tier": route_tier, "skipped": skipped,
    });
    if !c.session.is_empty() {
        app.sessions.lock().await.entry(c.session.clone()).or_default().orch_check = Some(record.clone());
    }
    if let Some(path) = &app.orch.log_path {
        let mut line = record.clone();
        line["ts"] = json!(log::now_ms() / 1000);
        line["session"] = json!(c.session);
        line["prompt"] = json!(c.prompt.chars().take(200).collect::<String>());
        log::append(path, &line);
    }
    Json(json!({ "trigger": trigger, "enabled": true, "multi_part_p": p, "cutoff": app.orch.cutoff, "planner": planner, "skipped": skipped }))
}

#[derive(serde::Deserialize)]
struct WorkerCheck {
    prompt: String,
    #[serde(default)]
    session: String,
    /// The Agent tool call this worker belongs to, as the mod reports it.
    #[serde(default)]
    tool_use_id: String,
}

/// `POST /jev/worker`, called by the mod as the planner spawns a worker: which model
/// does this worker's task need? `model` is null (leave the planner's choice) when
/// orchestration is off or Jev is down. Never fails the caller.
async fn worker_check(State(app): State<Arc<App>>, Json(c): Json<WorkerCheck>) -> Json<Value> {
    if !app.orch.enabled {
        return Json(json!({ "model": null }));
    }
    // Opus may work under a Fable planner only; the gateway knows which planner this turn has.
    let (planner, prompt_id) = {
        let sessions = app.sessions.lock().await;
        let st = sessions.get(&c.session);
        (st.and_then(|s| s.orch_planner), st.and_then(|s| s.prompt_id.clone()))
    };
    let allow_opus = planner == Some(MODEL_TOP);
    let judged = tokio::time::timeout(std::time::Duration::from_secs(10), app.router.worker_want(&c.prompt, allow_opus)).await.ok().flatten();
    let (model, reason, signals) = match judged {
        Some((m, r, s)) => (Some(m), r, s),
        None => (None, "jev unavailable: planner's choice stands".to_string(), Value::Null),
    };
    if let Some(path) = &app.orch.log_path {
        log::append(path, &json!({
            "kind": "worker", "ts": log::now_ms() / 1000, "session": c.session, "model": model, "planner": planner,
            "prompt_id": prompt_id, "tool_use_id": c.tool_use_id,
            "reason": reason, "signals": signals, "prompt": c.prompt.chars().take(200).collect::<String>(),
        }));
    }
    Json(json!({ "model": model, "reason": reason }))
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
        // The planner of the session's current turn when it is orchestrated: tags the main
        // thread's and the workers' rows alike, so a task's cost can be summed from them.
        let orch_planner = app.sessions.lock().await.get(&meta.session).and_then(|s| s.orch_planner);
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
                    "requested_model": requested, "model": model, "routed": picked.is_some(), "orch_planner": orch_planner,
                    "new_turn": picked.as_ref().is_some_and(|p| p.new_turn), "adapted": for_haiku,
                    "decision": picked.as_ref().and_then(|p| p.decision.clone()),
                    "stream": j.and_then(|j| j.get("stream")).and_then(Value::as_bool).unwrap_or(false),
                    "params": j.map(request_params),
                    "beta": header(&parts.headers, "anthropic-beta"),
                    "client": header(&parts.headers, "user-agent"),
                    "messages": j.and_then(|j| j.get("messages")).and_then(Value::as_array).map(Vec::len),
                    "tools": j.and_then(|j| j.get("tools")).and_then(Value::as_array).map(Vec::len),
                    "req_bytes": bytes.len(),
                    "prefix": j.map(|j| app.prefix.observe(&meta.session, &meta.class, j)),
                }),
                app.turns.clone(),
                meta.session.clone(),
                meta.prompt_id.clone(),
                meta.class.clone(),
            )
        });
    }

    let logged = tap.is_some();
    let (sent, retries) = send_upstream(app, &parts, &path_q, &bytes, for_haiku, logged).await;
    if let (Some(t), true) = (&mut tap, retries > 0) {
        t.set("retries", json!(retries));
    }
    let mut resp = match sent {
        Ok(r) => r,
        Err(e) => {
            let chain = error_chain(&e);
            eprintln!("[warn] upstream unreachable after {retries} retries: {chain}");
            if let Some(t) = &mut tap {
                t.error = Some(json!({ "type": "gateway_upstream", "message": chain }));
            }
            return Err(anyhow::Error::new(e).context("upstream request failed"));
        }
    };
    // Haiku has restrictions that can change without notice. Rather than lose a
    // background request, retry it once as sent by the client, on the main model.
    if let (Some(retry), true) = (fallback, haiku_declined(resp.status().as_u16())) {
        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap_or_default();
        let msg = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_default();
        eprintln!("[warn] {MODEL_CHEAP} rejected a request ({status}): {msg} — retrying on {MODEL_MID}");
        if let Some(t) = &mut tap {
            t.set("fallback_from", json!({ "model": MODEL_CHEAP, "status": status, "error": msg }));
            t.set("model", json!(MODEL_MID));
            t.set("adapted", json!(false));
        }
        resp = match send_upstream(app, &parts, &path_q, &retry, false, logged).await.0 {
            Ok(r) => r,
            Err(e) => {
                let chain = error_chain(&e);
                if let Some(t) = &mut tap {
                    t.error = Some(json!({ "type": "gateway_upstream", "message": chain }));
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

/// Pauses before each retry of a connection that could not be established.
const CONNECT_RETRY_DELAYS_MS: [u64; 2] = [250, 1000];

/// Send the request, retrying only when no connection was made. Then nothing reached
/// Anthropic, so a retry can't repeat work; once bytes are sent, a failure is passed
/// to Claude Code, which has its own retry. Returns the outcome and the retries used.
async fn send_upstream(
    app: &App,
    parts: &axum::http::request::Parts,
    path_q: &str,
    body: &[u8],
    for_haiku: bool,
    logged: bool,
) -> (Result<reqwest::Response, reqwest::Error>, u32) {
    let mut retries = 0;
    loop {
        match upstream_request(app, parts, path_q, body.to_vec(), for_haiku, logged).send().await {
            Err(e) if e.is_connect() && (retries as usize) < CONNECT_RETRY_DELAYS_MS.len() => {
                tokio::time::sleep(std::time::Duration::from_millis(CONNECT_RETRY_DELAYS_MS[retries as usize])).await;
                retries += 1;
            }
            other => return (other, retries),
        }
    }
}

/// An error and its causes, e.g. "error sending request: client error (Connect):
/// dns error: ...". reqwest's own message stops at the first, which hid why 155
/// requests failed.
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

/// A Haiku response that means "not this request, not now": the settings or size it
/// rejects (400, 413, 422) or its own rate limit (429, seen on title and summary requests
/// while the account still had quota on the other models). The request is retried on Sonnet.
fn haiku_declined(status: u16) -> bool {
    matches!(status, 400 | 413 | 422 | 429)
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
    // Only the user's own words are judged. Command echoes, `!` shell output and
    // notifications gave Jev random verdicts (120 of 2,123 logged decisions).
    let Some(words) = user_words(&prompt) else {
        return Some(same_turn(st));
    };
    let orch_check = st.orch_check.take();
    if is_orchestrated(req) {
        // The mod attached the orchestration brief: the main thread plans, delegates
        // and reviews on the planner; the workers it spawns name their own model.
        let planner = orch_check
            .as_ref()
            .and_then(|c| c["planner"].as_str())
            .map(|p| if p == MODEL_TOP { MODEL_TOP } else { app.orch.planner })
            .unwrap_or(app.orch.planner);
        if planner == MODEL_STRONG {
            // The context is now cached on Opus: let the usual step-down rule apply.
            st.sticky.tier = Some(2);
        } else if planner == MODEL_TOP {
            st.sticky.tier = Some(3);
        }
        st.prompt_id = prompt_id.clone();
        st.orch_planner = Some(planner);
        st.model = Some(planner);
        let decision = json!({ "tier": null, "reason": "orchestrate", "orchestrated": true, "confident": true, "orchestrate": orch_check });
        st.decision = Some(decision.clone());
        let prompt = strip_orchestrate_block(&prompt);
        log_decision(app, &session_id, prompt_id.as_deref(), planner, &decision, &prompt);
        return Some(Picked { model: planner, new_turn: true, decision: Some(decision) });
    }
    let context = recent_context(req);
    let fallback = st.sticky.tier.unwrap_or(1);
    drop(sessions); // don't hold the lock across the Jev call

    let want = app.router.code_want(&words, &context, fallback).await;
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(session_id.clone()).or_default();
    let (tier, reason) = st.sticky.decide(&want, app.downgrade_after);
    st.sticky.tier = Some(tier);
    st.prompt_id = prompt_id.clone();
    st.orch_planner = None;
    st.model = Some(TIERS[tier]);
    let decision = json!({ "tier": tier, "reason": reason, "confident": want.confident, "signals": want.signals, "orchestrate": orch_check });
    st.decision = Some(decision.clone());
    log_decision(app, &session_id, prompt_id.as_deref(), TIERS[tier], &decision, &words);
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

/// Background-task and subagent notices that Claude Code delivers as prompts. Kept out
/// of `is_harness_message` so routing is unchanged; the orchestrate check skips them.
fn is_notification(prompt: &str) -> bool {
    let p = prompt.trim_start();
    p.starts_with("<task-notification") || p.starts_with("<agent-message")
}

/// Whether the turn that this request starts carries the jev-orchestrate brief: the
/// marker in any user message after the last assistant one, reminders included (the
/// brief may arrive wrapped in one, or as its own block or message).
fn is_orchestrated(req: &Value) -> bool {
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
fn strip_orchestrate_block(prompt: &str) -> String {
    match prompt.find(ORCH_MARKER) {
        Some(i) => {
            let end = prompt[i..].find("</jev-orchestrate>").map(|e| i + e + "</jev-orchestrate>".len()).unwrap_or(prompt.len());
            format!("{}{}", &prompt[..i], &prompt[end..]).trim().to_string()
        }
        None => prompt.to_string(),
    }
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
    strip_tag(text, "system-reminder")
}

/// `text` without every `<tag>…</tag>` block. An unclosed block runs to the end
/// (prompts are cut at 4,000 characters, so a long one may lose its closing tag).
fn strip_tag(text: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(&close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text inside the first `<tag>…</tag>`, if there is one.
fn tag_text<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{tag}>"))?;
    Some(&text[start..start + end])
}

/// What the user actually typed in this prompt, for Jev to judge. None when there is
/// nothing of theirs: a background-task or subagent notice, a local command's echo
/// (`/cockpit`, `/clear`) or the output of a `!` shell command. A skill or custom
/// slash command counts as its name and arguments (`/better-ui wdyt so far?`), not
/// the skill text Claude Code appends. Pasted text is the user's and is kept.
fn user_words(prompt: &str) -> Option<String> {
    if is_notification(prompt) {
        return None;
    }
    let local_command = ["<local-command-caveat>", "<local-command-stdout>", "<local-command-stderr>"]
        .iter()
        .any(|t| prompt.contains(t));
    let mut text = prompt.to_string();
    for tag in ["local-command-caveat", "local-command-stdout", "local-command-stderr", "bash-input", "bash-stdout", "bash-stderr"] {
        text = strip_tag(&text, tag);
    }
    if let Some(name) = tag_text(&text, "command-name").map(|n| n.trim().to_string()) {
        if local_command {
            for tag in ["command-name", "command-message", "command-args"] {
                text = strip_tag(&text, tag);
            }
        } else {
            let args = tag_text(&text, "command-args").unwrap_or("").trim();
            text = format!("{name} {args}");
        }
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
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

/// A 1M-context session makes Claude Code add settings Haiku 4.5 rejected: the
/// long-context beta, adaptive thinking, effort, and mid-conversation system
/// messages. Haiku 5.5 accepted adaptive thinking, effort and context management
/// when Claude Code sent them directly (18 logged requests, all 200), but stripping
/// them from a title or summary request costs nothing, and the Sonnet fallback in
/// `forward` covers anything it still rejects.
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
    fn haiku_rejections_and_rate_limits_fall_back_to_sonnet() {
        for s in [400, 413, 422, 429] {
            assert!(haiku_declined(s), "{s}");
        }
        for s in [200, 401, 404, 500, 529] {
            assert!(!haiku_declined(s), "{s}");
        }
    }

    #[test]
    fn notifications_are_not_orchestrate_candidates() {
        assert!(is_notification("<task-notification>\n<task-id>a1</task-id>"));
        assert!(is_notification("  <agent-message from=\"a\">report"));
        assert!(!is_notification("why does the <task-notification> hook fire twice?"));
    }

    #[test]
    fn only_the_users_own_words_are_judged() {
        let caveat = "<local-command-caveat>The command below was run directly in Claude Code, not sent to you as a request.</local-command-caveat>";
        let echo = "<command-name>/cockpit</command-name>\n            <command-message>cockpit</command-message>\n            <command-args></command-args>\n\n<local-command-stdout>mech-hud: MECH-OS // COCKPIT ONLINE</local-command-stdout>";
        // A local command's echo alone: nothing to judge.
        assert_eq!(user_words(&format!("{caveat}\n{echo}")), None);
        assert_eq!(user_words(echo), None);
        // Echo followed by what the user typed: just their words.
        assert_eq!(
            user_words(&format!("{caveat}\n{echo}\nbefore start, think deeply about the execution")).as_deref(),
            Some("before start, think deeply about the execution")
        );
        // A skill command: its name and arguments, not the skill text.
        let skill = "<command-message>better-ui</command-message>\n<command-name>/better-ui</command-name>\n<command-args>wdyt of this implementation so far?</command-args>\n\nBase directory for this skill: /home/u/.claude/skills/better-ui\n# Better UI\nPolish the interface...";
        assert_eq!(user_words(skill).as_deref(), Some("/better-ui wdyt of this implementation so far?"));
        // Output of a `!` command is not a request.
        assert_eq!(user_words("<bash-input>cargo test</bash-input>\n\n<bash-stdout>ok</bash-stdout><bash-stderr></bash-stderr>"), None);
        assert_eq!(user_words("<bash-input>sudo pacman -Syu</bash-input>\n<bash-stdout>:: Proceed? [Y/n] Y\n:: Retrieving packages"), None, "unclosed (cut) output");
        // Notifications are never judged.
        assert_eq!(user_words("<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>"), None);
        // Pasted text and ordinary prompts are the user's own.
        let paste = "<pasted_content id=\"9462\">\nerror[E0308]: mismatched types\n</pasted_content>";
        assert_eq!(user_words(paste).as_deref(), Some(paste));
        assert_eq!(user_words("why does the <task-notification> hook fire twice?").as_deref(), Some("why does the <task-notification> hook fire twice?"));
        assert_eq!(user_words("   "), None);
        assert!(!is_notification("add audit logging across the API"));
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
    fn error_chain_includes_every_cause() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "error sending request")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused"));
        assert_eq!(error_chain(&e), "error sending request: connection refused");
    }

    #[tokio::test]
    async fn refused_connection_is_a_connect_error() {
        // Nothing listens on port 1, so every attempt fails to connect.
        let client = reqwest::Client::new();
        let err = client.get("http://127.0.0.1:1/").send().await.unwrap_err();
        assert!(err.is_connect(), "a refused connection is a connect error: {err}");
        assert!(error_chain(&err).contains("onnect"), "{}", error_chain(&err));
    }

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

    #[test]
    fn long_context_beta_is_stripped_from_the_list() {
        assert_eq!(
            without_long_context("claude-code-20250219,oauth-2025-04-20,context-1m-2025-08-07,interleaved-thinking-2025-05-14"),
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14"
        );
        assert_eq!(without_long_context("context-1m-2025-08-07"), "");
    }
}
