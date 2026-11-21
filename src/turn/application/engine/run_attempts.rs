//! The failover loop: choose a model, run it, react to how it ended, try the next.

use std::collections::HashSet;

use crate::backend::{Event, ProviderId, TurnError};
use crate::turn::domain::failover_policy::MAX_ATTEMPTS;
use crate::session::domain::turn_status::TurnStatus;
use crate::changes::domain::snapshot::Snapshot;
use crate::routing::domain::decide::NoRoute;
use crate::turn::application::cancel::Cancel;
use crate::turn::application::judged::Judged;
use crate::turn::application::ports::Ui;
use crate::turn::application::turn_report::TurnReport;
use crate::turn::domain::failover::{Failover, NoRouteInfo};
use super::attempt::{with_steered, Attempt};
use super::choose_model::ChooseInput;
use super::Engine;

impl Engine {
    pub(super) async fn run_attempts(
        &mut self,
        prompt: &str,
        judged: Judged,
        ui: &mut dyn Ui,
        cancel: &Cancel,
        before: &mut Option<Snapshot>,
    ) -> TurnReport {
        let signals = judged.signals();
        let jev = judged.log_signals();
        // Legacy mode is the old single-provider path: one attempt, no failover.
        let max_attempts = if matches!(judged, Judged::Providers(_)) { self.guards.failover.max_attempts.min(MAX_ATTEMPTS) } else { 1 };
        // Every attempt decides from the state the turn started with, so the hold
        // counter advances once per user turn however many models fail.
        let base = self.routing.clone();
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        let mut notes: Vec<String> = Vec::new();
        let mut last_failure: Option<(Attempt, TurnError)> = None;
        let mut attempts = 0;
        // What the person added while the turn ran, kept for a retry on another model.
        let mut steered: Vec<String> = Vec::new();
        loop {
            if cancel.is_cancelled() {
                // The earlier attempt really failed (and may have touched files): keep its
                // own error, then add the interruption, so neither is lost.
                if let Some((failed, err)) = &last_failure {
                    self.record_failed(prompt, failed, err);
                }
                return self.cancelled_report(prompt, last_failure.as_ref().map(|(a, _)| a), attempts, ui);
            }
            let now = (self.now)();
            let chosen = match self.choose(ChooseInput {
                judged: &judged,
                signals: signals.as_ref(),
                prompt,
                base: &base,
                excluded: &excluded,
                attempts,
                now,
            }) {
                Ok(c) => c,
                Err(e) => {
                    let message = self.no_route_message(&e, now, &excluded);
                    match &e {
                        NoRoute::PinUnusable { what } => ui.no_route(&NoRouteInfo::Pin { what }, &message),
                        NoRoute::NothingUsable { task } => {
                            ui.no_route(&NoRouteInfo::Nothing { task: &task.to_string() }, &message)
                        }
                        NoRoute::Refused { why } => ui.no_route(&NoRouteInfo::Refused { why }, &message),
                    }
                    if let Some((failed, err)) = last_failure {
                        self.record_failed(prompt, &failed, &err);
                    }
                    return TurnReport::ended(TurnStatus::NoRoute, message, attempts);
                }
            };
            attempts += 1;
            if attempts == 1 {
                // The state the first model starts from, so a failover's changes count too.
                *before = self.snapshot_tree(cancel, true).await;
            }
            ui.route_facts(&chosen.facts);
            // What was added during an earlier attempt goes to the next model only if the policy
            // lets that model have it.
            let provider = chosen.spec.provider;
            let (allowed, kept_out): (Vec<String>, Vec<String>) = steered.iter().cloned().partition(|t| self.guards.text_permitted(&self.dir, t, provider));
            // They left the queue when the first model took them: say what did not follow.
            for text in &kept_out {
                let shown = text.lines().next().unwrap_or_default();
                ui.note(&format!("not passed on to {provider} (your privacy rules), so this model does not have what you added during the turn: {shown}"));
            }
            let attempt = self.run_attempt(chosen, &with_steered(prompt, &allowed), ui, cancel).await;
            steered.extend(attempt.steered.iter().cloned());
            self.turn_activity.add_tools(&attempt.tools);
            self.log_attempt(&attempt, &jev, attempts, &notes);
            if !attempt.cancelled {
                match &attempt.result {
                    Ok(_) => self.guards.scoreboard.record_answer(attempt.chosen.spec.provider, attempt.wall_ms),
                    Err(e) => self.guards.scoreboard.record_failure(attempt.chosen.spec.provider, &e.kind, attempt.wall_ms),
                }
            }
            if attempt.cancelled {
                return self.cancelled_report(&with_steered(prompt, &steered), Some(&attempt), attempts, ui);
            }
            let err = match attempt.result.clone() {
                Ok(out) => {
                    // A model cannot come back by itself: say so when it left work running or
                    // promised to report later.
                    if !attempt.background_noticed {
                        let running = crate::turn::domain::background::spawned(&attempt.tools) + attempt.background_results;
                        if let Some(notice) = crate::turn::domain::background::notice(running, crate::turn::domain::background::promises_update(&out.text)) {
                            ui.event(&Event::Hint(notice));
                        }
                    }
                    return self.finish(&with_steered(prompt, &steered), attempt, out, attempts);
                }
                Err(err) => err,
            };
            let spec = attempt.chosen.spec;
            // A fresh reading: the attempt may have run for minutes, and "resets in 3h20m"
            // counts from when the error arrived, not from when the attempt began.
            let failed_at = (self.now)();
            let retry_note = self.runtime.react(spec.provider, spec.id, &err, &mut excluded, failed_at);
            match retry_note {
                Some(note) if attempts < max_attempts => {
                    let note = if attempt.discarded {
                        format!("{note} (output of the failed attempt discarded)")
                    } else {
                        note
                    };
                    ui.failover(&Failover {
                        provider: spec.provider,
                        model: spec.id,
                        kind: &err.kind,
                        message: &err.message,
                        discarded: attempt.discarded,
                        will_retry: true,
                        text: &note,
                    });
                    notes.push(note);
                    last_failure = Some((attempt, err));
                }
                retryable => {
                    let tail = match (retryable, attempts) {
                        (Some(_), n) if n > 1 => format!(" (stopped after {n} attempts; try again or see /providers)"),
                        _ => String::new(),
                    };
                    let message = format!("[error] {} {} failed: {}{tail}", spec.provider, spec.id, err.message);
                    ui.failover(&Failover {
                        provider: spec.provider,
                        model: spec.id,
                        kind: &err.kind,
                        message: &err.message,
                        discarded: attempt.discarded,
                        will_retry: false,
                        text: &message,
                    });
                    self.record_failed(&with_steered(prompt, &steered), &attempt, &err);
                    return TurnReport {
                        provider: Some(spec.provider),
                        model: Some(spec.id.to_string()),
                        tool_calls: attempt.tools.len() as u32,
                        ..TurnReport::ended(TurnStatus::Failed, message, attempts)
                    };
                }
            }
        }
    }
}
