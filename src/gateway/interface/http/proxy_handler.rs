//! The catch-all route: forwards every request to Anthropic, swapping in the routed model
//! for `jev-code` and streaming the response back unchanged while the usage tap reads it.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, Method, Response, StatusCode};
use futures_util::StreamExt as _;
use serde_json::{json, Value};

use crate::catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID};
use crate::gateway::application::app_state::App;
use crate::gateway::application::turns::Turns;
use crate::gateway::application::usage_tap::UsageTap;
use crate::gateway::domain::request_signals::{find_cwd, project_of, request_params};
use crate::gateway::domain::haiku_adaptation::{adapt_for_haiku, haiku_declined};
use crate::gateway::domain::prompt_text::{last_user_prompt, recent_context};
use crate::gateway::infrastructure::anthropic_wire::set_model;
use crate::shared::infrastructure::jsonl_log as log;

use super::choose_model::{choose_model, Meta};
use super::upstream::{error_chain, send_upstream, SKIP_HEADERS};

pub(super) const MAX_BODY: usize = 64 * 1024 * 1024;

pub(super) async fn proxy(State(app): State<Arc<App>>, req: Request) -> Response<Body> {
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

pub(super) async fn forward(app: &Arc<App>, req: Request) -> Result<Response<Body>> {
    let (parts, body) = req.into_parts();
    let path_q = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let mut bytes = to_bytes(body, MAX_BODY).await.context("read request body")?;
    let started = Instant::now();
    let ts_ms = log::now_ms();

    // Only real message requests are logged; token counting and the rest pass through.
    let mut tap = None;
    let mut for_haiku = false;
    // The request as sent by the client, on the main model: retried if Haiku rejects it.
    let mut fallback: Option<Bytes> = None;
    if should_route_messages(&parts.method, parts.uri.path(), &parts.headers) {
        let meta = Meta::from_headers(&parts.headers);
        let tracking = app.sessions.lock().await.entry(&meta.session, &app.turns_path).tracking.clone();
        let json: Option<Value> = serde_json::from_slice(&bytes).ok();
        let requested = json.as_ref().and_then(|j| j.get("model")).and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(j) = &json {
            let prompt = last_user_prompt(j);
            let new_turn = tracking.turns.observe(&meta.session, meta.prompt_id.as_deref(), &meta.class, j, &requested, prompt.as_deref(), ts_ms);
            if let (true, Some(prev), Some(pid), Some(prompt)) = (correction_check_enabled(), new_turn.and_then(|n| n.prev_prompt), &meta.prompt_id, prompt) {
                tracking.turns.begin_check(&meta.session, pid);
                spawn_outcome_check(app, tracking.turns.clone(), meta.session.clone(), pid.clone(), prompt, prev, recent_context(j));
            }
        }
        let picked = match &json {
            Some(j) => choose_model(app, &meta, j).await,
            None => None,
        };
        if let (Some(p), Some(pid)) = (&picked, &meta.prompt_id) {
            if let (true, Some(d)) = (p.new_turn, &p.decision) {
                tracking.turns.set_decision(&meta.session, pid, d.clone());
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
                bytes = serde_json::to_vec(&adapted).map(Bytes::from).unwrap_or(bytes);
                for_haiku = true;
            }
        }
        tap = app.usage_path.clone().map(|path| {
            let j = json.as_ref();
            let cwd = tracking.turns.cwd(&meta.session).or_else(|| j.and_then(find_cwd));
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
                    "prefix": j.map(|j| tracking.prefix.observe(&meta.session, match meta.class.as_str() { "main" | "auxiliary" | "subagent" => &meta.class, _ => "other" }, j)),
                }),
                tracking.turns.clone(),
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

/// The LLM-judged `correction_p` is opt-in (`JEV_CORRECTION_CHECK=1`): on 276 logged
/// turns it did not correlate with aborts, quick follow-ups or model switches, so it
/// is not worth a classifier call per turn until it is validated against real labels.
fn correction_check_enabled() -> bool {
    std::env::var("JEV_CORRECTION_CHECK").is_ok_and(|v| v == "1")
}

/// Background: ask Jev whether the user's new prompt reacts to the last answer as
/// wrong or incomplete. The result lands in the turn's `followup.correction_p`; the
/// turn row waits for it. Never delays or affects routing.
fn spawn_outcome_check(app: &Arc<App>, turns: Arc<Turns>, session: String, prompt_id: String, prompt: String, prev_prompt: String, prev_answer: String) {
    let app = app.clone();
    tokio::spawn(async move {
        let check = app.router.is_correction(&prompt, &prev_prompt, &prev_answer);
        let p = tokio::time::timeout(std::time::Duration::from_secs(20), check).await.ok().and_then(Result::ok);
        turns.finish_check(&session, &prompt_id, "correction_p", p.map(|p| json!(p)));
    });
}

fn has_credentials(headers: &HeaderMap) -> bool {
    headers.contains_key("x-api-key") || headers.contains_key("authorization")
}

fn should_route_messages(method: &Method, path: &str, headers: &HeaderMap) -> bool {
    *method == Method::POST && path == "/v1/messages" && has_credentials(headers)
}

pub(super) fn header<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_messages_skip_routing_and_tracking() {
        let mut headers = HeaderMap::new();
        assert!(!should_route_messages(&Method::POST, "/v1/messages", &headers));
        headers.insert("x-api-key", "key".parse().unwrap());
        assert!(should_route_messages(&Method::POST, "/v1/messages", &headers));
        assert!(!should_route_messages(&Method::GET, "/v1/messages", &headers));
        assert!(!should_route_messages(&Method::POST, "/jev/worker", &headers));
        headers.remove("x-api-key");
        headers.insert("authorization", "Bearer token".parse().unwrap());
        assert!(should_route_messages(&Method::POST, "/v1/messages", &headers));
    }
}
