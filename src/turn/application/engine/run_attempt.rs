//! Running one attempt: catch the model up, start it, forward its events.

use std::sync::Mutex;
use std::time::Instant;

use crate::backend::{
    self, ErrorKind, Event, ProviderId, TurnError,
    TurnRequest,
};
use crate::turn::domain::cache;
use crate::approval::application::extension_hooks as hooks;
use crate::orchestration::domain::orchestration_gate as orchestrate;
use crate::turn::application::approve_tool::{lock_ui, EngineApprover, SharedUi};
use crate::turn::application::cancel::Cancel;
use crate::turn::application::ports::Ui;
use super::attempt::{Attempt, Chosen};
use super::{Engine, CATCH_UP_TOKENS};

impl Engine {
    /// The prompt for `provider`: with the catch-up in front when its own session
    /// missed turns that ran elsewhere. This does not mark anything as delivered: the
    /// provider has only seen the catch-up once an attempt ends successfully
    /// ([`Engine::finish`] records the turn, which marks it caught up). An attempt
    /// that fails first leaves the turns missed, so the next one sends them again.
    pub(super) fn prompt_for(&mut self, provider: ProviderId, model: &str, prompt: &str) -> String {
        // The privacy check's snapshot, when it took one, so what was checked is what is sent.
        let snapshot = self.git_snapshot.take();
        if self.sessions.missed_turns(provider) == 0 {
            return prompt.to_string();
        }
        let git = snapshot.unwrap_or_else(|| (self.git_status)(&self.dir));
        match self.sessions.catch_up_for(provider, model, &git, CATCH_UP_TOKENS) {
            Some(text) => format!("{text}{prompt}"),
            None => prompt.to_string(),
        }
    }

    pub(super) async fn run_attempt(&mut self, chosen: Chosen, prompt: &str, ui: &mut dyn Ui, cancel: &Cancel) -> Attempt {
        let provider = chosen.spec.provider;
        // Secrets are blanked after the catch-up text is added, so earlier turns are scanned too.
        let sent = self.prompt_for(provider, chosen.spec.id, prompt);
        let mut sent = self.guards.outgoing(&sent, provider, chosen.spec.id, (self.now)()).prompt;
        let mut backend = (self.backend_factory)(provider);
        if let (Some(warm), Some(est)) = (self.routing.warm, cache::switch_estimate(self.routing.warm.as_ref(), provider, (self.now)())) {
            ui.note(&cache::switch_note(&est, warm.provider, provider));
        }
        if let Some(effort) = self.effort.resolve(chosen.spec.tier) {
            backend.set_effort(effort);
        }
        backend.set_streaming(self.stream_text);
        if let Some(mcp) = chosen.mcp.as_ref().filter(|_| backend.supports_mcp()) {
            match orchestrate::write_mcp_config(mcp) {
                Ok(()) => {
                    backend.attach_mcp(mcp);
                    sent = format!("{}\n\n{sent}", orchestrate::PLANNER_NOTE);
                }
                Err(e) => ui.note(&format!("[orchestrate] could not write {}: {e}; running without workers", mcp.config_path.display())),
            }
        }
        // Claude Code (and DeepSeek through it) has this in its system note; the other CLIs
        // take none, so it follows the prompt.
        if !matches!(provider, ProviderId::Claude | ProviderId::DeepSeek) {
            sent = crate::turn::domain::background::with_note(&sent);
        }
        let req = TurnRequest {
            dir: self.dir.clone(),
            model: chosen.spec.id.to_string(),
            prompt: sent,
            images: self.images.clone(),
            session_id: self.sessions.session_id(provider).map(String::from),
            mode: self.mode,
            trust_project_settings: hooks::trust::project_settings_trusted(&self.dir),
        };
        let mut tools: Vec<String> = Vec::new();
        let mut steered: Vec<String> = Vec::new();
        let (mut background_results, mut background_noticed) = (0usize, false);
        let steer_rx = if backend.steerable() { ui.steering() } else { None };
        let steer_wanted = if steer_rx.is_some() { ui.steer_wanted() } else { None };
        // Progress (tools, plan) is shown as it happens; assistant text is held back until
        // the attempt is known to have succeeded, so a failed attempt's partial commentary
        // never sits on screen as if it were an answer.
        let mut held: Vec<Event> = Vec::new();
        // Whether any text was shown before the attempt was known to have succeeded, so a
        // failed attempt can say it discarded it.
        let mut streamed_any = false;
        let idle = self.idle_timeout;
        let stream = self.stream_text;
        let started = Instant::now();
        let runtime = &mut self.runtime;
        let result = {
            let shared_ui: SharedUi = Mutex::new(&mut *ui);
            let approver = EngineApprover { ui: &shared_ui, cancel, dir: &self.dir, mode: self.mode, hooks: &self.hooks, allowed_tools: &self.allowed_tools };
            let hooks = &self.hooks;
            let (guards, now, dir, model) = (&self.guards, &self.now, &self.dir, chosen.spec.id);
            let prepare = move |text: &str| {
                if steer_wanted.as_ref().is_some_and(|wanted| !wanted(text)) {
                    return None;
                }
                guards.steer_text(dir, text, provider, model, now())
            };
            let steering = steer_rx.map(|rx| backend::Steering { rx, prepare: &prepare });
            let mut on_event = |e: &Event| match e {
                Event::Text(_) => {
                    if stream {
                        lock_ui(&shared_ui).event(e);
                    }
                    held.push(e.clone());
                }
                // Partial text is a preview of the answer, not an answer: the whole text
                // still arrives above (or in the finished event) and is what gets committed.
                Event::TextDelta(_) => {
                    if stream {
                        lock_ui(&shared_ui).event(e);
                        streamed_any = true;
                    }
                }
                other => {
                    lock_ui(&shared_ui).event(other);
                    match other {
                        Event::Tool(summary) => {
                            tools.push(summary.clone());
                            let name = summary.split_whitespace().next();
                            hooks::runner::fire(hooks, hooks::Event::PostTool, name, Some(summary));
                        }
                        Event::Plan(plan) => runtime.note_plan(provider, plan.clone()),
                        Event::Steered(text) => steered.push(text.clone()),
                        Event::ToolResult { text, .. } if crate::turn::domain::background::result_reports_background(text) => background_results += 1,
                        Event::Hint(t) if t.starts_with(crate::turn::domain::background::NOTICE_PREFIX) => background_noticed = true,
                        _ => {}
                    }
                }
            };
            // Dropping the turn future on a cancel kills the CLI and its children.
            tokio::select! {
                r = backend::run_turn_steered(&*backend, &req, idle, &mut on_event, &approver, steering) => Some(r),
                _ = cancel.cancelled() => None,
            }
        };
        let cancelled = result.is_none();
        runtime.after_turn(provider);
        let result = result.unwrap_or_else(|| Err(TurnError::new(ErrorKind::Other, "cancelled by the user")));
        let discarded = result.is_err() && (!held.is_empty() || streamed_any);
        if let Ok(out) = &result {
            let streamed = |t: &str| {
                if held.iter().any(|e| matches!(e, Event::Text(s) if s.trim() == t)) {
                    return true;
                }
                let mut remaining = t;
                for e in &held {
                    if let Event::Text(s) = e {
                        let Some(rest) = remaining.trim_start().strip_prefix(s.trim()) else { return false };
                        remaining = rest;
                    }
                }
                !held.is_empty() && remaining.trim().is_empty()
            };
            // Some CLIs deliver the answer only in the final event; show it unless it was already among the texts.
            let answer = out.text.trim();
            let missing = !answer.is_empty() && !streamed(answer);
            if !stream {
                for e in &held {
                    ui.event(e);
                }
            }
            if missing {
                ui.event(&Event::Text(answer.to_string()));
            }
        }
        Attempt { chosen, tools, steered, wall_ms: started.elapsed().as_millis() as u64, result, discarded, cancelled, background_results, background_noticed }
    }
}
