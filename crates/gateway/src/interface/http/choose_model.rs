//! Which real model a `jev-code` request runs on, decided once per user prompt.

use axum::http::HeaderMap;
use serde_json::{json, Value};

use jev_catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP};
use crate::application::app_state::App;
use crate::application::session_state::SessionState;
use crate::domain::model_choice::{ALIAS, TIERS};
use crate::domain::orchestration_brief::strip_orchestrate_block;
use crate::domain::prompt_text::{is_harness_message, user_words};
use crate::infrastructure::orchestration_brief::is_orchestrated;
use crate::infrastructure::prompt_text::{last_user_prompt, recent_context};
use jev_shared::infrastructure::jsonl_log as log;

use super::proxy_handler::header;

/// Claude Code's per-request hint headers.
pub(super) struct Meta {
    pub(super) class: String,
    pub(super) session: String,
    pub(super) prompt_id: Option<String>,
}

impl Meta {
    pub(super) fn from_headers(h: &HeaderMap) -> Self {
        Self {
            class: header(h, "x-claude-code-request-class").unwrap_or("main").to_string(),
            session: header(h, "x-claude-code-session-id").unwrap_or("default").to_string(),
            prompt_id: header(h, "x-claude-code-prompt-id").map(str::to_string),
        }
    }
}

/// A routed request: the real model, and the turn's decision for the usage log.
pub(super) struct Picked {
    pub(super) model: &'static str,
    pub(super) auxiliary_one_shot: bool,
    pub(super) new_turn: bool,
    pub(super) decision: Option<Value>,
}

/// The real model for a `jev-code` request, or None to forward it untouched.
pub(super) async fn choose_model(app: &App, meta: &Meta, req: &Value) -> Option<Picked> {
    if req.get("model")?.as_str()? != ALIAS {
        return None;
    }
    let (class, session_id, prompt_id) = (meta.class.as_str(), meta.session.clone(), meta.prompt_id.clone());
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(&session_id, &app.turns_path);
    let same_turn = |st: &SessionState| Picked {
        model: st.model.unwrap_or(MODEL_MID),
        auxiliary_one_shot: false,
        new_turn: false,
        decision: st.decision.clone(),
    };

    if class == "auxiliary" {
        // A fork of the conversation (carries the tool list) shares the main model's
        // prompt cache and can exceed Haiku's 200k window; only small one-shot
        // requests (titles, summaries) go to Haiku.
        let fork = req.get("tools").and_then(Value::as_array).is_some_and(|t| !t.is_empty());
        return Some(if fork { same_turn(st) } else { Picked { model: MODEL_CHEAP, auxiliary_one_shot: true, new_turn: false, decision: None } });
    }
    if class == "subagent" {
        // Subagents arriving as jev-code (the main model named no model) are mostly
        // searches and reads. Sonnet for every request keeps a run on one model, so a
        // main-model switch can't rebuild the subagent's cache mid-run. An explicit
        // model on the Agent tool never reaches here: only the alias is routed.
        return Some(Picked { model: MODEL_MID, auxiliary_one_shot: false, new_turn: false, decision: None });
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
        return Some(Picked { model: planner, auxiliary_one_shot: false, new_turn: true, decision: Some(decision) });
    }
    let context = recent_context(req);
    let fallback = st.sticky.tier.unwrap_or(1);
    drop(sessions); // don't hold the lock across the Jev call

    let want = app.router.code_want(&words, &context, fallback).await;
    let mut sessions = app.sessions.lock().await;
    let st = sessions.entry(&session_id, &app.turns_path);
    let (tier, reason) = st.sticky.decide(&want, app.downgrade_after);
    st.sticky.tier = Some(tier);
    st.prompt_id = prompt_id.clone();
    st.orch_planner = None;
    st.model = Some(TIERS[tier]);
    let decision = json!({ "tier": tier, "reason": reason, "confident": want.confident, "signals": want.signals.to_json(), "orchestrate": orch_check });
    st.decision = Some(decision.clone());
    log_decision(app, &session_id, prompt_id.as_deref(), TIERS[tier], &decision, &words);
    Some(Picked { model: TIERS[tier], auxiliary_one_shot: false, new_turn: true, decision: Some(decision) })
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
