//! Per-turn rollup: requests roll up into one record per user turn, with how the
//! user reacted to the previous turn (gap, rephrase, model switch).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::domain::request_signals::{project_of, similarity};
use jev_shared::application::host::Host;

/// Token fields summed into turn records.
const TOKEN_FIELDS: [&str; 4] =
    ["input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"];

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
    /// The log writer and clock, shared with this session's usage taps.
    pub(super) host: Arc<dyn Host>,
}

impl Turns {
    pub fn new(path: Option<String>, host: Arc<dyn Host>) -> Arc<Self> {
        Arc::new(Self { path, inner: Mutex::new(TurnsInner::default()), host })
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
        find_cwd: impl FnOnce() -> Option<String>,
        requested_model: &str,
        prompt: Option<&str>,
        now_ms: u64,
    ) -> Option<NewTurn> {
        let mut g = self.lock();
        if g.sessions.get(session).is_none_or(|s| s.cwd.is_none()) {
            if let Some(c) = find_cwd() {
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
        self.host.append_line(
            path,
            &json!({
                "ts_ms": a.ts_ms, "end_ms": a.last_end_ms, "wall_ms": a.last_end_ms.saturating_sub(a.ts_ms),
                "session": a.session, "prompt_id": a.prompt_id, "turn_index": a.turn_index,
                "cwd": a.cwd, "project": a.cwd.as_deref().map(project_of),
                "requested_model": a.requested_model, "prompt_chars": a.prompt_chars,
                "decision": a.decision, "followup": a.followup,
                "requests": a.requests, "by_class": a.by_class, "models": a.models,
                "tokens": a.tokens, "tools": a.tools, "errors": a.errors, "ended": ended,
            })
            .to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_learned_mid_turn_reaches_the_open_turn() {
        let turns = Turns::new(None, Arc::new(jev_shared::infrastructure::system_host::SystemHost));
        let usage = Map::new();
        turns.observe("s", Some("p1"), "main", || None, "m", Some("hi"), 1);
        turns.observe("s", Some("p1"), "subagent", || Some("/home/u/app".to_string()), "m", None, 2);
        assert_eq!(turns.lock().open.values().next().unwrap().cwd.as_deref(), Some("/home/u/app"));
        turns.finish_request(end("p1", &usage, "end_turn"));
    }

    fn end<'a>(pid: &'a str, usage: &'a Map<String, Value>, stop: &'a str) -> ReqEnd<'a> {
        ReqEnd {
            session: "s", prompt_id: Some(pid), class: "main", model: "m", usage, tools: &[],
            failed: false, stop_reason: Some(stop), complete: true, end_ms: 10,
        }
    }

    #[test]
    fn turn_stays_open_across_tool_calls_and_closes_on_end_turn() {
        let turns = Turns::new(None, Arc::new(jev_shared::infrastructure::system_host::SystemHost));
        let usage = Map::new();
        assert!(turns.observe("s", Some("p1"), "main", || None, "m", Some("hi"), 1).is_some());
        // Tool-result requests of the same turn are not a new turn.
        assert!(turns.observe("s", Some("p1"), "main", || None, "m", None, 2).is_none());
        turns.finish_request(end("p1", &usage, "tool_use"));
        assert_eq!(turns.lock().open.len(), 1);
        turns.finish_request(end("p1", &usage, "end_turn"));
        assert!(turns.lock().open.is_empty());
    }

    #[test]
    fn closed_turn_waits_for_pending_check_and_new_turn_sweeps_the_old() {
        let turns = Turns::new(None, Arc::new(jev_shared::infrastructure::system_host::SystemHost));
        let usage = Map::new();
        turns.observe("s", Some("p1"), "main", || None, "m", Some("hi"), 1);
        turns.observe("s", Some("p2"), "main", || None, "m", Some("again"), 2);
        assert_eq!(turns.lock().open.len(), 1, "p1 swept when p2 began");
        turns.begin_check("s", "p2");
        turns.finish_request(end("p2", &usage, "end_turn"));
        assert_eq!(turns.lock().open.len(), 1, "held for the pending check");
        turns.finish_check("s", "p2", "correction_p", Some(json!(0.9)));
        assert!(turns.lock().open.is_empty());
    }
}
