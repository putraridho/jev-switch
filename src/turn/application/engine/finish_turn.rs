//! Ending a turn: record it, log it, build the report.

use std::collections::HashSet;

use serde_json::Value;

use crate::backend::{ProviderId, TurnError, TurnOutcome};
use crate::session::domain::turn_status::TurnStatus;
use crate::turn::domain::cache::WarmCache;
use crate::catalog::domain::model_table as catalog;
use crate::session::domain::turn_record::TurnRecord;
use crate::routing::domain::decide as routing;
use crate::routing::domain::decide::NoRoute;
use crate::turn::application::ports::Ui;
use crate::turn::application::turn_report::TurnReport;
use super::attempt::Attempt;
use super::Engine;
use crate::turn::application::ports::AttemptRow;

impl Engine {
    pub(super) fn finish(&mut self, prompt: &str, attempt: Attempt, out: TurnOutcome, attempts: u32) -> TurnReport {
        let Attempt { chosen, tools, .. } = attempt;
        let tool_calls = tools.len() as u32;
        let spec = chosen.spec;
        if let Some(id) = out.session_id.clone() {
            self.sessions.set_session_id(spec.provider, id);
        }
        self.sessions.record(TurnRecord {
            prompt: prompt.to_string(),
            provider: spec.provider,
            model: spec.id.to_string(),
            reply: out.text.clone(),
            tools,
            ok: true,
        });
        self.routing = chosen.state;
        routing::record_turn(&mut self.routing, spec);
        self.routing.warm = Some(WarmCache::after_turn(spec.provider, &out.usage, (self.now)()));
        if let Some(tier) = chosen.legacy_tier {
            self.sticky.tier = Some(tier);
        }
        self.previous = Some((prompt.to_string(), out.text.clone()));
        self.last_context_tokens = out.usage.context_tokens.unwrap_or(0);
        self.guards.meter.record(spec.provider, spec.id, &out.usage, &self.guards.prices);
        TurnReport {
            status: TurnStatus::Answered,
            text: out.text,
            message: String::new(),
            provider: Some(spec.provider),
            model: Some(spec.id.to_string()),
            usage: Some(out.usage),
            attempts,
            tool_calls,
            changes: None,
        }
    }

    /// End a turn the user stopped. If a model had been started, the turn is remembered
    /// as interrupted so a later catch-up (or the same model's next prompt) knows files
    /// may be half changed. Nothing else moves: no failover, no cooldown, no routing state.
    pub(super) fn cancelled_report(&mut self, prompt: &str, attempt: Option<&Attempt>, attempts: u32, ui: &mut dyn Ui) -> TurnReport {
        let message = "[cancelled] stopped by the user; the turn was not retried".to_string();
        ui.cancelled(&message);
        let Some(attempt) = attempt else { return TurnReport::ended(TurnStatus::Cancelled, message, attempts) };
        let spec = attempt.chosen.spec;
        // Not `record_failed`: that would mark the running provider caught up, but its own
        // CLI session never recorded the killed turn, so it must be told next time too.
        self.sessions.record_interrupted(prompt, spec.provider, spec.id, attempt.tools.clone());
        TurnReport {
            provider: Some(spec.provider),
            model: Some(spec.id.to_string()),
            tool_calls: attempt.tools.len() as u32,
            ..TurnReport::ended(TurnStatus::Cancelled, message, attempts)
        }
    }

    /// Remember a turn that got no answer, so a later catch-up says it was asked.
    pub(super) fn record_failed(&mut self, prompt: &str, attempt: &Attempt, err: &TurnError) {
        self.sessions.record_failed(TurnRecord {
            prompt: prompt.to_string(),
            provider: attempt.chosen.spec.provider,
            model: attempt.chosen.spec.id.to_string(),
            reply: format!("(no answer: {})", err.message),
            tools: attempt.tools.clone(),
            ok: false,
        });
    }

    /// Append this attempt's row to the turn log, when there is one.
    pub(super) fn log_attempt(&self, a: &Attempt, jev: &Value, attempt: u32, notes: &[String]) {
        let Some(path) = &self.turn_log else { return };
        let spec = a.chosen.spec;
        let session = a
            .result
            .as_ref()
            .ok()
            .and_then(|o| o.session_id.as_deref())
            .or(self.sessions.session_id(spec.provider));
        let row = AttemptRow {
            provider: spec.provider,
            model: spec.id,
            session,
            reason: &a.chosen.reason,
            jev,
            result: &a.result,
            wall_ms: a.wall_ms,
            attempt,
            orchestrated: a.chosen.mcp.is_some(),
            cancelled: a.cancelled,
            notes,
        };
        self.turn_log_writer.append(path, &row);
    }

    /// Plain words for "no model can take this turn", never a guess at one.
    pub(super) fn no_route_message(&self, e: &NoRoute, now: u64, excluded: &HashSet<(ProviderId, String)>) -> String {
        match e {
            NoRoute::Refused { why } => format!("[error] Nothing was sent: {why}."),
            NoRoute::PinUnusable { what } => format!(
                "[error] {what}. A /model pin is never swapped for another model: use /model auto, or /providers to see what is available."
            ),
            NoRoute::NothingUsable { task } => {
                let usable = self.runtime.usable(now).without(excluded);
                let mut ready: Vec<ProviderId> = Vec::new();
                for m in catalog::models().iter().filter(|m| usable.contains(m)) {
                    if !ready.contains(&m.provider) {
                        ready.push(m.provider);
                    }
                }
                let ready = if ready.is_empty() {
                    "none".to_string()
                } else {
                    ready.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
                };
                let security = if *task == catalog::TaskType::Security {
                    " Security work never goes to deepseek or grok."
                } else {
                    ""
                };
                let failed = if excluded.is_empty() { "" } else { " Models that failed this turn are skipped for the rest of it." };
                format!(
                    "[error] Nothing can take this {task} turn.{security}{failed} Providers able to run now: {ready}. \
                     /providers shows why the others cannot (not installed, not logged in, cooling down after a limit, refused); \
                     fix that, /providers refresh, and send it again."
                )
            }
        }
    }
}
