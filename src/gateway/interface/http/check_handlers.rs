//! `POST /jev/orchestrate` and `POST /jev/worker`: the checks the jev-orchestrate mod calls.

use std::sync::Arc;

use axum::extract::{Json, State};
use serde_json::{json, Value};

use crate::catalog::domain::model_table::MODEL_TOP;
use crate::gateway::application::app_state::App;
use crate::gateway::domain::prompt_text::{is_harness_message, user_words};
use crate::judgment::domain::orchestration::Orchestration;
use crate::orchestration::domain::orchestration_gate::{orch_gate, Gate};
use crate::shared::infrastructure::jsonl_log as log;

#[derive(serde::Deserialize)]
pub(super) struct OrchCheck {
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) context: String,
    #[serde(default)]
    pub(super) session: String,
}

/// `POST /jev/orchestrate`, called by the jev-orchestrate mod as a prompt is
/// submitted: should this prompt be orchestrated? Never fails the caller: if Jev is
/// down the answer is no.
pub(super) async fn orchestrate_check(State(app): State<Arc<App>>, Json(c): Json<OrchCheck>) -> Json<Value> {
    if !app.orch.enabled {
        return Json(json!({ "trigger": false, "enabled": false }));
    }
    // Not typed by the user: no Jev call, no trigger (the mod skips these too).
    let words = if is_harness_message(&c.prompt) { None } else { user_words(&c.prompt) };
    let Some(words) = words else {
        return Json(json!({ "trigger": false, "enabled": true, "skipped": "not typed" }));
    };
    let check = app.router.orchestration_p(&words, &c.context);
    let judgment = match tokio::time::timeout(std::time::Duration::from_secs(10), check).await {
        Ok(Ok(j)) => {
            if j.sequential_p.is_none() {
                eprintln!("[warn] orchestrate check: jev's sequential answer was unreadable; no veto this time");
            }
            Some(j)
        }
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
    if judgment.as_ref().is_some_and(|j| j.passes(app.orch.cutoff)) {
        let want = app.router.code_want(&words, &c.context, 1).await;
        if !want.signals.is_null() {
            route_tier = Some(want.tier);
            hard_p = want.signals.to_json()["hard_p"].as_f64();
        }
    }
    let gate = orch_gate(judgment.as_ref(), app.orch.cutoff, route_tier, app.orch.require_opus);
    let trigger = gate == Gate::Trigger;
    let planner = if trigger && !app.orch.planner_pinned && route_tier.is_some_and(|t| t >= 3) { MODEL_TOP } else { app.orch.planner };
    let skipped = (gate == Gate::SingleModelRoute).then_some("normal routing would not use Opus");
    let record = check_record(
        judgment.as_ref(),
        json!({ "trigger": trigger, "cutoff": app.orch.cutoff, "planner": planner, "hard_p": hard_p, "route_tier": route_tier, "skipped": skipped }),
    );
    if !c.session.is_empty() {
        app.sessions.lock().await.entry(&c.session, &app.turns_path).orch_check = Some(record.clone());
    }
    if let Some(path) = &app.orch.log_path {
        let mut line = record.clone();
        line["ts"] = json!(log::now_ms() / 1000);
        line["session"] = json!(c.session);
        line["prompt"] = json!(c.prompt.chars().take(200).collect::<String>());
        log::append(path, &line);
    }
    Json(check_record(
        judgment.as_ref(),
        json!({ "trigger": trigger, "enabled": true, "cutoff": app.orch.cutoff, "planner": planner, "skipped": skipped }),
    ))
}

/// An orchestrate check's log row and response: Jev's judgment (score, the raw
/// `multi_part_p`, `sequential_p`, `vetoed`; `orch_report` and `orch_tune` read the raw
/// values to see what the sequence veto held back) beside the gateway's own `rest`.
fn check_record(judgment: Option<&Orchestration>, rest: Value) -> Value {
    // All null when Jev gave no answer. The raw probabilities and the veto are all logged, so
    // a report can say what a cutoff would have triggered and what the veto held back.
    let mut record = json!({
        "multi_part_p": judgment.map(|j| j.multi_part_p),
        "sequential_p": judgment.map(|j| j.sequential_p),
        "vetoed": judgment.map(|j| j.vetoed),
    });
    if let (Some(fields), Value::Object(rest)) = (record.as_object_mut(), rest) {
        fields.extend(rest);
    }
    record
}

#[derive(serde::Deserialize)]
pub(super) struct WorkerCheck {
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) session: String,
    /// The Agent tool call this worker belongs to, as the mod reports it.
    #[serde(default)]
    pub(super) tool_use_id: String,
}

/// `POST /jev/worker`, called by the mod as the planner spawns a worker: which model
/// does this worker's task need? `model` is null (leave the planner's choice) when
/// orchestration is off or Jev is down. Never fails the caller.
pub(super) async fn worker_check(State(app): State<Arc<App>>, Json(c): Json<WorkerCheck>) -> Json<Value> {
    if !app.orch.enabled {
        return Json(json!({ "model": null }));
    }
    // Opus may work under a Fable planner only; the gateway knows which planner this turn has.
    let (planner, prompt_id) = {
        let mut sessions = app.sessions.lock().await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_orchestrate_check_logs_the_raw_judgment_and_that_it_was_vetoed() {
        // Multi-part 0.95 but a sequence: the veto refuses it at any cutoff, and the log
        // still shows the raw 0.95 beside `vetoed`.
        let vetoed = Orchestration::new(0.95, 0.9);
        let record = check_record(Some(&vetoed), json!({ "trigger": false, "cutoff": 0.7 }));
        assert_eq!(record["multi_part_p"], 0.95);
        assert!(record.get("orch_score").is_none(), "no capped score is logged any more");
        assert_eq!(record["sequential_p"], 0.9);
        assert_eq!(record["vetoed"], true);
        assert_eq!((record["trigger"].clone(), record["cutoff"].clone()), (json!(false), json!(0.7)));
        let failed = check_record(None, json!({ "trigger": false }));
        assert!(failed["multi_part_p"].is_null() && failed["vetoed"].is_null());
    }
}
