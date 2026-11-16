//! One coding turn, end to end: ask Jev what the turn is, pick a model that is really
//! usable, bring that model up to date on what the others did, run it, and if it
//! fails in a way another provider can fix, try the next one.
//!
//! The policy lives elsewhere ([`crate::routing`] picks, [`crate::catalog`] knows the
//! models, [`crate::handoff`] writes the catch-up, [`crate::providers`] knows who is
//! usable). This module only sequences them, so each step stays testable on its own
//! and the `jev-code` binary is a thin shell that prints what a [`Ui`] is handed.
//!
//! Two hard rules hold on every path:
//! - a provider that is not usable is never started (the choice always goes through
//!   [`routing::decide`] with the live usable set, also after a failure);
//! - a failed attempt never produces an answer: only a successful attempt returns text.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::backend::{self, Backend, Event, McpAttachment, ProviderId, TurnError, TurnOutcome, TurnRequest, Usage};
use crate::catalog::{self, find, ModelSpec};
use crate::handoff::{Sessions, TurnRecord};
use crate::log;
use crate::orchestrate::{self, Outcome, Setup};
use crate::providers::{self, Runtime};
use crate::router::{CodeWant, ProviderWant, Router, Sticky};
use crate::routing::{self, NoRoute, Pin, RouteDecision, RouteInput, RoutingState, Signals};
use crate::turnlog::turn_row;

/// Most models tried for one user turn before giving up.
pub const MAX_ATTEMPTS: u32 = 6;
/// Budget for the catch-up block put in front of a prompt after a provider switch.
const CATCH_UP_TOKENS: usize = 10_000;
/// The Claude models of the legacy (routing off) tiers, cheapest first.
const LEGACY_TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];

/// Where a turn shows its progress. The binary prints; tests record.
/// `Send` because the runner calls back from inside its own future.
pub trait Ui: Send {
    /// The routing decision, one line, before the turn runs.
    fn route(&mut self, line: &str);
    /// Streamed progress: tool calls, assistant text, permission hints.
    fn event(&mut self, e: &Event);
    /// Anything else the user should know: failover, errors, "nothing is usable".
    fn note(&mut self, line: &str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// A model answered; see [`TurnReport::text`].
    Answered,
    /// No usable model could take the turn, so nothing was started.
    NoRoute,
    /// A model was started and failed in a way that ended the turn.
    Failed,
}

#[derive(Debug, Clone)]
pub struct TurnReport {
    pub status: TurnStatus,
    /// The answer; empty unless `status` is [`TurnStatus::Answered`].
    pub text: String,
    /// What the user was told when the turn did not end in an answer.
    pub message: String,
    /// The provider and model that answered, or that failed last.
    pub provider: Option<ProviderId>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    /// Models started for this turn.
    pub attempts: u32,
}

impl TurnReport {
    fn ended(status: TurnStatus, message: String, attempts: u32) -> Self {
        Self { status, text: String::new(), message, provider: None, model: None, usage: None, attempts }
    }
}

/// What Jev (or the legacy tier judgment) said about one turn.
pub enum Judged {
    /// Provider routing; `None` when Jev was unavailable.
    Providers(Option<ProviderWant>),
    /// Routing off: the old Claude-only tier judgment.
    Legacy(CodeWant),
}

impl Judged {
    fn signals(&self) -> Option<Signals> {
        match self {
            Self::Providers(Some(w)) => Some(Signals::from(w)),
            _ => None,
        }
    }

    /// Jev's numbers for the turn log.
    fn log_signals(&self) -> Value {
        match self {
            Self::Providers(Some(w)) => w.signals.clone(),
            Self::Providers(None) => Value::Null,
            Self::Legacy(w) => w.signals.clone(),
        }
    }
}

pub type BackendFactory = Box<dyn Fn(ProviderId) -> Box<dyn Backend> + Send + Sync>;
type GitStatus = Box<dyn Fn(&Path) -> String + Send + Sync>;
type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// `git status --short` in `dir`; empty on any failure (not a repo, no git).
pub fn git_status_short(dir: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--short"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Longest silence from a CLI before its turn is cut off, so a hung CLI cannot
/// freeze the session (`CODE_IDLE_TIMEOUT_SECS`, default 900).
pub fn idle_timeout_from_env() -> Duration {
    let secs = std::env::var("CODE_IDLE_TIMEOUT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(900);
    Duration::from_secs(secs)
}

/// The model chosen for one attempt.
struct Chosen {
    spec: &'static ModelSpec,
    reason: String,
    /// The routing state after the decision; kept only if the attempt succeeds.
    state: RoutingState,
    /// Legacy mode only: the tier index, to feed back into [`Sticky`].
    legacy_tier: Option<usize>,
    /// Orchestration: the delegate tool to attach, when this model is the planner.
    mcp: Option<McpAttachment>,
}

/// One started attempt and what came of it.
struct Attempt {
    chosen: Chosen,
    tools: Vec<String>,
    wall_ms: u64,
    result: Result<TurnOutcome, TurnError>,
}

pub struct Engine {
    pub runtime: Runtime,
    pub sessions: Sessions,
    pub routing: RoutingState,
    router: Router,
    pin: Pin,
    dir: PathBuf,
    enabled: bool,
    backend_factory: BackendFactory,
    git_status: GitStatus,
    now: Clock,
    idle_timeout: Duration,
    turn_log: Option<String>,
    /// The last (prompt, reply), for Jev's correction question.
    previous: Option<(String, String)>,
    /// Context size the last answering CLI reported.
    last_context_tokens: u64,
    /// Legacy (routing off) stickiness.
    sticky: Sticky,
    downgrade_after: u32,
    /// `/orchestrate`: off until asked for, because a planner plus workers costs more
    /// than one model on a small task.
    orchestrate: bool,
    /// Where the delegate tool is; `None` means orchestration cannot attach it.
    orch_setup: Option<Setup>,
}

impl Engine {
    /// Real backends, the real clock and git, and the turn log from `JEV_CODE_LOG`.
    /// Routing is on.
    pub fn new(runtime: Runtime, router: Router, dir: PathBuf) -> Self {
        Self {
            runtime,
            sessions: Sessions::new(),
            routing: RoutingState::default(),
            router,
            pin: Pin::Auto,
            dir,
            enabled: true,
            backend_factory: Box::new(providers::backend_for),
            git_status: Box::new(git_status_short),
            now: Box::new(log::now_ms),
            idle_timeout: idle_timeout_from_env(),
            turn_log: log::path_from_env("JEV_CODE_LOG", "code-turns.jsonl"),
            previous: None,
            last_context_tokens: 0,
            sticky: Sticky::default(),
            downgrade_after: 2,
            orchestrate: false,
            orch_setup: None,
        }
    }

    /// Start with orchestration on or off (the binary reads `JEV_ORCHESTRATE`).
    pub fn with_orchestrate(mut self, on: bool) -> Self {
        self.orchestrate = on;
        self
    }

    /// Where the `delegate_mcp` binary and the MCP config directory are.
    pub fn with_orch_setup(mut self, setup: Option<Setup>) -> Self {
        self.orch_setup = setup;
        self
    }

    pub fn with_backend_factory(mut self, f: BackendFactory) -> Self {
        self.backend_factory = f;
        self
    }

    pub fn with_git_status(mut self, f: GitStatus) -> Self {
        self.git_status = f;
        self
    }

    pub fn with_clock(mut self, f: Clock) -> Self {
        self.now = f;
        self
    }

    pub fn with_idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    pub fn with_turn_log(mut self, path: Option<String>) -> Self {
        self.turn_log = path;
        self
    }

    pub fn with_enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Easier turns before the legacy path steps back down a tier.
    pub fn with_downgrade_after(mut self, n: u32) -> Self {
        self.downgrade_after = n;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn pin(&self) -> &Pin {
        &self.pin
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_pin(&mut self, pin: Pin) {
        self.pin = pin;
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn orchestrate(&self) -> bool {
        self.orchestrate
    }

    pub fn set_orchestrate(&mut self, on: bool) {
        self.orchestrate = on;
    }

    /// `/new`: forget the conversation and every provider session. The pin, the
    /// routing switch and what is known about providers stay.
    pub fn new_session(&mut self) {
        self.sessions.reset();
        self.routing = RoutingState::default();
        self.sticky = Sticky::default();
        self.previous = None;
        self.last_context_tokens = 0;
    }

    /// The text of `/providers`.
    pub fn providers_text(&self) -> String {
        self.runtime.render((self.now)())
    }

    /// The compact view printed at startup and after `/providers refresh`.
    pub fn summary_text(&self) -> String {
        self.runtime.render_summary((self.now)())
    }

    /// `/providers refresh`: probe again.
    pub async fn refresh_providers(&mut self) {
        self.runtime.refresh().await;
    }

    /// Run one user turn.
    pub async fn run(&mut self, prompt: &str, ui: &mut dyn Ui) -> TurnReport {
        let judged = self.judge(prompt, ui).await;
        self.run_judged(prompt, judged, ui).await
    }

    /// Ask Jev. The only network call of a turn before the CLI starts.
    async fn judge(&self, prompt: &str, ui: &mut dyn Ui) -> Judged {
        let recent = self.sessions.recent_context(3, 300);
        if !self.enabled {
            let want = self.router.code_want(prompt, &recent, self.sticky.tier.unwrap_or(1)).await;
            if want.reason.starts_with("jev unavailable") {
                ui.note(&format!("[jev] {}; keeping current model", want.reason));
            }
            return Judged::Legacy(want);
        }
        let previous = self.previous.as_ref().map(|(p, r)| (p.as_str(), r.as_str()));
        let want = self.router.provider_want(prompt, &recent, previous).await;
        if want.is_none() {
            ui.note("[jev] unavailable (timeout or error); routing without its judgment");
        }
        Judged::Providers(want)
    }

    /// Run a turn whose Jev judgment is already in hand: choose, hand off, run, and
    /// fail over. Split from [`Engine::run`] so everything after the network call is
    /// testable with fake backends.
    pub async fn run_judged(&mut self, prompt: &str, judged: Judged, ui: &mut dyn Ui) -> TurnReport {
        let signals = judged.signals();
        let jev = judged.log_signals();
        // Legacy mode is the old single-provider path: one attempt, no failover.
        let max_attempts = if matches!(judged, Judged::Providers(_)) { MAX_ATTEMPTS } else { 1 };
        // Every attempt decides from the state the turn started with, so the hold
        // counter advances once per user turn however many models fail.
        let base = self.routing.clone();
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        let mut notes: Vec<String> = Vec::new();
        let mut last_failure: Option<(Attempt, TurnError)> = None;
        let mut attempts = 0;
        loop {
            let now = (self.now)();
            let chosen = match self.choose(&judged, signals.as_ref(), prompt, &base, &excluded, now) {
                Ok(c) => c,
                Err(e) => {
                    let message = self.no_route_message(&e, now, &excluded);
                    ui.note(&message);
                    if let Some((failed, err)) = last_failure {
                        self.record_failed(prompt, &failed, &err);
                    }
                    return TurnReport::ended(TurnStatus::NoRoute, message, attempts);
                }
            };
            attempts += 1;
            ui.route(&chosen.reason);
            let attempt = self.run_attempt(chosen, prompt, ui).await;
            self.log_attempt(&attempt, &jev, attempts, &notes);
            let err = match attempt.result.clone() {
                Ok(out) => return self.finish(prompt, attempt, out, attempts),
                Err(err) => err,
            };
            let spec = attempt.chosen.spec;
            let retry_note = self.runtime.react(spec.provider, spec.id, &err, &mut excluded, now);
            match retry_note {
                Some(note) if attempts < max_attempts => {
                    ui.note(&note);
                    notes.push(note);
                    last_failure = Some((attempt, err));
                }
                retryable => {
                    let tail = match (retryable, attempts) {
                        (Some(_), n) if n > 1 => format!(" (stopped after {n} attempts; try again or see /providers)"),
                        _ => String::new(),
                    };
                    let message = format!("[error] {} {} failed: {}{tail}", spec.provider, spec.id, err.message);
                    ui.note(&message);
                    self.record_failed(prompt, &attempt, &err);
                    return TurnReport {
                        provider: Some(spec.provider),
                        model: Some(spec.id.to_string()),
                        ..TurnReport::ended(TurnStatus::Failed, message, attempts)
                    };
                }
            }
        }
    }

    // ------------------------------------------------------------------ choosing

    fn choose(
        &mut self,
        judged: &Judged,
        signals: Option<&Signals>,
        prompt: &str,
        base: &RoutingState,
        excluded: &HashSet<(ProviderId, String)>,
        now: u64,
    ) -> Result<Chosen, NoRoute> {
        if let Judged::Legacy(want) = judged {
            return Ok(self.legacy_choice(want, base));
        }
        let usable = self.runtime.usable(now).without(excluded);
        let context_tokens = (self.sessions.context_tokens_estimate() as u64).max(self.last_context_tokens);
        let plan = |p: ProviderId| self.runtime.plan_utilization(p);
        let input = RouteInput {
            signals,
            prompt_words: prompt.split_whitespace().count(),
            pin: &self.pin,
            usable: &usable,
            allow: &self.runtime.allow,
            overrides: &self.runtime.overrides,
            context_tokens,
            plan_utilization: &plan,
        };
        let mut state = base.clone();
        let decision = routing::decide(&mut state, &input)?;
        let outcome = self.plan_orchestration(signals, &decision, &input, base);
        let (spec, reason, mcp) = self.apply_orchestration(outcome, &decision);
        Ok(Chosen { spec, reason, state, legacy_tier: None, mcp })
    }

    /// Providers whose CLI can have the delegate tool attached. A backend is cheap to
    /// build and says it itself, so no list of providers is kept here to drift.
    fn mcp_providers(&self) -> Vec<ProviderId> {
        providers::ALL_PROVIDERS.into_iter().filter(|p| (self.backend_factory)(*p).supports_mcp()).collect()
    }

    /// Should this turn have a planner? The decision is [`orchestrate::choose_planner`];
    /// this supplies what it reads. The planner is the normal route's model when its CLI
    /// can take the tool, else the best Top model among the providers that can (the same
    /// routing, with only those providers usable, so a `/model` pin is still respected).
    fn plan_orchestration(
        &self,
        signals: Option<&Signals>,
        normal: &RouteDecision,
        input: &RouteInput,
        base: &RoutingState,
    ) -> Outcome {
        let tool_ready = self.orch_setup.as_ref().is_some_and(|s| s.bin.is_some());
        let mcp = if self.orchestrate { self.mcp_providers() } else { Vec::new() };
        orchestrate::choose_planner(
            self.orchestrate,
            signals.map(|s| s.multi_part_p),
            normal.spec,
            tool_ready,
            &mcp,
            |keep| {
                let restricted = input.usable.only_providers(keep);
                let mut state = base.clone();
                routing::decide(&mut state, &RouteInput { usable: &restricted, ..*input }).ok().map(|d| d.spec)
            },
        )
    }

    /// The model, the `[route]` line and the tool to attach once orchestration has had its say.
    fn apply_orchestration(&self, outcome: Outcome, normal: &RouteDecision) -> (&'static ModelSpec, String, Option<McpAttachment>) {
        match outcome {
            Outcome::Normal => (normal.spec, normal.reason.clone(), None),
            Outcome::Unavailable(why) => (normal.spec, format!("{}; orchestration skipped: {why}", normal.reason), None),
            Outcome::Planner(spec) => {
                let mcp = self.orch_setup.as_ref().and_then(|s| s.attachment(&self.dir, spec));
                let moved = if spec == normal.spec {
                    String::new()
                } else {
                    format!(" ({} cannot take the delegate tool)", normal.spec.provider)
                };
                let reason = format!("{}; orchestrating: {} {} plans{moved}, Jev picks the workers", normal.reason, spec.provider, spec.id);
                (spec, reason, mcp)
            }
        }
    }

    /// Routing off: Claude only, a `/model` pin of a Claude model or else the
    /// upgrade-now, downgrade-slowly tier rule. Availability is not consulted; this is
    /// the old behaviour on purpose.
    fn legacy_choice(&mut self, want: &CodeWant, base: &RoutingState) -> Chosen {
        let ignored = match &self.pin {
            Pin::Provider(p) | Pin::Model(p, _) if *p != ProviderId::Claude => {
                format!(" (pin {} ignored: routing is off, claude only)", self.pin)
            }
            _ => String::new(),
        };
        let pinned = match &self.pin {
            Pin::Model(ProviderId::Claude, id) => find(ProviderId::Claude, id),
            _ => None,
        };
        let (spec, why) = match pinned {
            Some(spec) => (spec, "pinned with /model".to_string()),
            None => {
                let (tier, why) = self.sticky.decide(want, self.downgrade_after);
                (legacy_spec(tier), why)
            }
        };
        let legacy_tier = LEGACY_TIERS.iter().position(|id| *id == spec.id);
        Chosen { spec, reason: format!("{} {} — {why}{ignored}", spec.provider, spec.id), state: base.clone(), legacy_tier, mcp: None }
    }

    // ------------------------------------------------------------------ running

    /// The prompt for `provider`: with the catch-up in front when its own session
    /// missed turns that ran elsewhere.
    fn prompt_for(&mut self, provider: ProviderId, prompt: &str) -> String {
        if self.sessions.missed_turns(provider) == 0 {
            return prompt.to_string();
        }
        let git = (self.git_status)(&self.dir);
        let catch_up = self.sessions.catch_up(provider, &git, CATCH_UP_TOKENS);
        self.sessions.mark_seen(provider);
        match catch_up {
            Some(text) => format!("{text}{prompt}"),
            None => prompt.to_string(),
        }
    }

    async fn run_attempt(&mut self, chosen: Chosen, prompt: &str, ui: &mut dyn Ui) -> Attempt {
        let provider = chosen.spec.provider;
        let mut sent = self.prompt_for(provider, prompt);
        let mut backend = (self.backend_factory)(provider);
        if let Some(mcp) = chosen.mcp.as_ref().filter(|_| backend.supports_mcp()) {
            match orchestrate::write_mcp_config(mcp) {
                Ok(()) => {
                    backend.attach_mcp(mcp);
                    sent = format!("{}\n\n{sent}", orchestrate::PLANNER_NOTE);
                }
                Err(e) => ui.note(&format!("[orchestrate] could not write {}: {e}; running without workers", mcp.config_path.display())),
            }
        }
        let req = TurnRequest {
            dir: self.dir.clone(),
            model: chosen.spec.id.to_string(),
            prompt: sent,
            session_id: self.sessions.session_id(provider).map(String::from),
        };
        let mut tools: Vec<String> = Vec::new();
        let mut said: Vec<String> = Vec::new();
        let idle = self.idle_timeout;
        let started = Instant::now();
        let runtime = &mut self.runtime;
        let result = {
            let mut on_event = |e: &Event| {
                ui.event(e);
                match e {
                    Event::Tool(summary) => tools.push(summary.clone()),
                    Event::Text(t) if !t.starts_with('\u{2717}') => said.push(t.trim().to_string()),
                    Event::Plan(plan) => runtime.note_plan(provider, plan.clone()),
                    _ => {}
                }
            };
            backend::run_turn(&*backend, &req, idle, &mut on_event).await
        };
        // Some CLIs deliver the answer only in the final event; show it unless it was already streamed.
        if let Ok(out) = &result {
            if !out.text.trim().is_empty() && !said.iter().any(|t| *t == out.text.trim()) {
                ui.event(&Event::Text(out.text.trim().to_string()));
            }
        }
        Attempt { chosen, tools, wall_ms: started.elapsed().as_millis() as u64, result }
    }

    // ------------------------------------------------------------------ outcomes

    fn finish(&mut self, prompt: &str, attempt: Attempt, out: TurnOutcome, attempts: u32) -> TurnReport {
        let Attempt { chosen, tools, .. } = attempt;
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
        if let Some(tier) = chosen.legacy_tier {
            self.sticky.tier = Some(tier);
        }
        self.previous = Some((prompt.to_string(), out.text.clone()));
        self.last_context_tokens = out.usage.context_tokens.unwrap_or(0);
        TurnReport {
            status: TurnStatus::Answered,
            text: out.text,
            message: String::new(),
            provider: Some(spec.provider),
            model: Some(spec.id.to_string()),
            usage: Some(out.usage),
            attempts,
        }
    }

    /// Remember a turn that got no answer, so a later catch-up says it was asked.
    fn record_failed(&mut self, prompt: &str, attempt: &Attempt, err: &TurnError) {
        self.sessions.record(TurnRecord {
            prompt: prompt.to_string(),
            provider: attempt.chosen.spec.provider,
            model: attempt.chosen.spec.id.to_string(),
            reply: format!("(no answer: {})", err.message),
            tools: attempt.tools.clone(),
            ok: false,
        });
    }

    fn log_attempt(&self, a: &Attempt, jev: &Value, attempt: u32, notes: &[String]) {
        let Some(path) = &self.turn_log else { return };
        let spec = a.chosen.spec;
        let session = a
            .result
            .as_ref()
            .ok()
            .and_then(|o| o.session_id.as_deref())
            .or(self.sessions.session_id(spec.provider));
        let mut row = turn_row(spec.provider, spec.id, session, &a.chosen.reason, jev, &a.result, a.wall_ms);
        if let Value::Object(o) = &mut row {
            o.insert("attempt".into(), json!(attempt));
            o.insert("orchestrated".into(), json!(a.chosen.mcp.is_some()));
            if !notes.is_empty() {
                o.insert("failover_notes".into(), json!(notes));
            }
        }
        log::append(path, &row);
    }

    /// Plain words for "no model can take this turn", never a guess at one.
    fn no_route_message(&self, e: &NoRoute, now: u64, excluded: &HashSet<(ProviderId, String)>) -> String {
        match e {
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

fn legacy_spec(tier: usize) -> &'static ModelSpec {
    let id = LEGACY_TIERS[tier.min(LEGACY_TIERS.len() - 1)];
    find(ProviderId::Claude, id).expect("legacy tiers are catalog models")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::process::Command;

    use crate::availability::{Availability, State};
    use crate::backend::{classify_common, ErrorKind, Finish};
    use crate::catalog::{default_chain, pick, Allowances, TaskType, Usable};
    use crate::router::Security;
    use crate::typesafe::TypeSafe;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    const NOW: u64 = 1_760_000_000_000;
    const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:the answer\\n'";
    const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";
    const NOT_ENTITLED: &str = "printf 'FAIL:model not available on your plan\\n'";
    const AUTH: &str = "printf 'FAIL:401 Unauthorized\\n'";
    const EMPTY: &str = "printf 'FIN:\\n'";
    const OTHER: &str = "printf 'FAIL:segmentation fault\\n'";
    const STALL: &str = "sleep 5";

    /// A "CLI" that is a shell script, with a grammar of canned lines (see backend tests).
    struct Fake {
        provider: ProviderId,
        script: &'static str,
        started: Arc<Mutex<Vec<Started>>>,
        attached: Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>,
    }

    #[derive(Debug, Clone)]
    struct Started {
        provider: ProviderId,
        model: String,
        prompt: String,
        session: Option<String>,
    }

    impl Backend for Fake {
        fn provider(&self) -> ProviderId {
            self.provider
        }
        fn command(&self, req: &TurnRequest) -> Command {
            self.started.lock().unwrap().push(Started {
                provider: self.provider,
                model: req.model.clone(),
                prompt: req.prompt.clone(),
                session: req.session_id.clone(),
            });
            let mut c = Command::new("sh");
            c.arg("-c").arg(self.script).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            let line = line.trim_end();
            let ev = match line.split_once(':') {
                Some(("FIN", t)) => Event::Finished(Finish {
                    text: t.into(),
                    session_id: Some(format!("sess-{}", self.provider)),
                    usage: Usage { output_tokens: 7, context_tokens: Some(1000), ..Usage::default() },
                }),
                Some(("FAIL", m)) => Event::Failed { message: m.into() },
                Some(("TOOL", t)) => Event::Tool(t.into()),
                _ => return vec![],
            };
            vec![ev]
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            classify_common(message).unwrap_or(ErrorKind::Other)
        }
        /// Like the real CLIs: claude, deepseek (through claude) and codex can; gemini and grok cannot.
        fn supports_mcp(&self) -> bool {
            matches!(self.provider, Claude | DeepSeek | Codex)
        }
        fn attach_mcp(&mut self, mcp: &McpAttachment) {
            self.attached.lock().unwrap().push((self.provider, mcp.clone()));
        }
    }

    #[derive(Default)]
    struct Recorder {
        routes: Vec<String>,
        texts: Vec<String>,
        notes: Vec<String>,
    }

    impl Ui for Recorder {
        fn route(&mut self, line: &str) {
            self.routes.push(line.into());
        }
        fn event(&mut self, e: &Event) {
            if let Event::Text(t) = e {
                self.texts.push(t.clone());
            }
        }
        fn note(&mut self, line: &str) {
            self.notes.push(line.into());
        }
    }

    struct Rig {
        engine: Engine,
        scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>>,
        started: Arc<Mutex<Vec<Started>>>,
        attached: Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>,
        factory_calls: Arc<Mutex<Vec<ProviderId>>>,
        clock: Arc<AtomicU64>,
        ui: Recorder,
    }

    fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    fn rig_with(states: &[(ProviderId, State)], script: &'static str) -> Rig {
        let avail = states.iter().map(|(p, s)| availability(*p, *s)).collect();
        let router = Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0);
        let scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>> = Arc::new(Mutex::new(
            [Claude, Codex, Gemini, Grok, DeepSeek].into_iter().map(|p| (p, script)).collect(),
        ));
        let started = Arc::new(Mutex::new(Vec::new()));
        let attached = Arc::new(Mutex::new(Vec::new()));
        let factory_calls = Arc::new(Mutex::new(Vec::new()));
        let clock = Arc::new(AtomicU64::new(NOW));
        let (s, st, fc, ck, at) =
            (scripts.clone(), started.clone(), factory_calls.clone(), clock.clone(), attached.clone());
        let engine = Engine::new(Runtime::for_tests(avail), router, std::env::temp_dir())
            .with_backend_factory(Box::new(move |p| {
                fc.lock().unwrap().push(p);
                let script = s.lock().unwrap()[&p];
                Box::new(Fake { provider: p, script, started: st.clone(), attached: at.clone() })
            }))
            .with_git_status(Box::new(|_| " M src/a.rs\n".to_string()))
            .with_clock(Box::new(move || ck.load(Ordering::SeqCst)))
            .with_idle_timeout(Duration::from_millis(300))
            .with_turn_log(None);
        Rig { engine, scripts, started, attached, factory_calls, clock, ui: Recorder::default() }
    }

    fn rig(ready: &[ProviderId]) -> Rig {
        let all = [Claude, DeepSeek, Codex, Gemini, Grok];
        let states: Vec<_> =
            all.iter().map(|p| (*p, if ready.contains(p) { State::Ready } else { State::NotLoggedIn })).collect();
        rig_with(&states, OK)
    }

    impl Rig {
        fn script(&self, p: ProviderId, s: &'static str) {
            self.scripts.lock().unwrap().insert(p, s);
        }

        async fn turn(&mut self, prompt: &str, w: ProviderWant) -> TurnReport {
            self.engine.run_judged(prompt, Judged::Providers(Some(w)), &mut self.ui).await
        }

        fn started(&self) -> Vec<Started> {
            self.started.lock().unwrap().clone()
        }
    }

    fn want(task: TaskType) -> ProviderWant {
        ProviderWant {
            task,
            task_confidence: 0.9,
            very_hard_p: 0.0,
            security_p: 0.0,
            edit_p: 0.3,
            continues_p: 0.9,
            correction_p: None,
            multi_part_p: 0.0,
            security_keywords: Security::None,
            signals: json!({ "task": task.as_str() }),
        }
    }

    /// A confident new task, which is what lets routing change provider.
    fn new_task(task: TaskType) -> ProviderWant {
        ProviderWant { continues_p: 0.1, ..want(task) }
    }

    const LONG: &str = "please look at the whole module and change how errors are reported to callers";

    /// The provider the catalog puts first for `task` when only `ready` are usable.
    fn first_choice(task: TaskType, ready: &[ProviderId]) -> ProviderId {
        let usable = Usable::all_of(ready.iter().copied());
        pick(&default_chain(task, false), task, &usable, &Allowances::default(), 0).expect("usable").spec.provider
    }

    #[tokio::test]
    async fn quota_cools_the_provider_reruns_the_same_prompt_and_skips_it_next_turn() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        let b = if a == Claude { Codex } else { Claude };
        r.script(a, QUOTA);

        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(report.provider, Some(b));
        assert_eq!(report.text, "the answer");
        assert_eq!(report.attempts, 2);
        let s = r.started();
        assert_eq!(s.iter().map(|x| x.provider).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(s[1].prompt, "add a retry to the client", "the same prompt, nothing to catch up on yet");
        assert!(r.engine.runtime.cooldowns.active(a, NOW).is_some());
        assert!(r.ui.notes.iter().any(|n| n.contains("hit its limit") && n.contains("trying the next model")));
        // The failed attempt's output is never the answer.
        assert!(r.ui.texts.iter().all(|t| t == "the answer"));

        let again = r.turn("and add a test", want(TaskType::Feature)).await;
        assert_eq!(again.provider, Some(b));
        assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 1, "A is skipped next turn");
    }

    #[tokio::test]
    async fn not_entitled_marks_the_model_unusable_and_never_retries_it() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, NOT_ENTITLED);

        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_ne!(report.provider, Some(a));
        let refused: Vec<_> = r.engine.runtime.unusable_models.iter().cloned().collect();
        assert!(refused.iter().all(|(p, _)| *p == a) && !refused.is_empty(), "{refused:?}");

        r.turn(LONG, new_task(TaskType::Feature)).await;
        for pair in &refused {
            assert_eq!(r.started().iter().filter(|x| (x.provider, x.model.clone()) == *pair).count(), 1, "{pair:?}");
        }
    }

    #[tokio::test]
    async fn empty_and_stalled_exclude_for_this_turn_only() {
        for (script, expect) in [(EMPTY, "empty answer"), (STALL, "stopped answering")] {
            let ready = [Claude, Codex];
            let mut r = rig(&ready);
            let a = first_choice(TaskType::Feature, &ready);
            r.script(a, script);
            let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
            assert_eq!(report.status, TurnStatus::Answered, "{script}");
            assert_ne!(report.provider, Some(a));
            assert!(r.ui.notes.iter().any(|n| n.contains(expect) || n.contains("empty")), "{:?}", r.ui.notes);
            // Nothing lasting: no cooldown, no refused model, and A can serve the next turn.
            assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
            assert!(r.engine.runtime.unusable_models.is_empty());
            r.script(a, OK);
            let next = r.turn(LONG, new_task(TaskType::Feature)).await;
            assert_eq!(next.provider, Some(a), "{script}: A is usable again next turn");
        }
    }

    #[tokio::test]
    async fn other_errors_stop_at_once_and_are_remembered_as_failed() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        let b = if a == Claude { Codex } else { Claude };
        r.script(a, OTHER);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Failed);
        assert_eq!(report.attempts, 1);
        assert!(report.text.is_empty());
        assert_eq!(r.started().len(), 1, "no retry on another provider");
        assert!(report.message.contains("segmentation fault"));
        let catch_up = r.engine.sessions.catch_up(b, "", 10_000).expect("the failed turn is on record");
        assert!(catch_up.contains("FAILED") && catch_up.contains("add a retry to the client"));
    }

    #[tokio::test]
    async fn auth_failure_blocks_the_provider_for_the_session_and_names_the_tool_to_log_in_to() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, AUTH);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        // A login problem is not a usage limit: it is kept out of the persisted cooldowns,
        // blocked for this session, and cleared by `/providers refresh`.
        assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
        assert!(r.engine.runtime.auth_failed.contains(&a));
        assert!(r.ui.notes.iter().any(|n| n.contains("log in to") && n.contains(providers::login_target(a))), "{:?}", r.ui.notes);
    }

    #[tokio::test]
    async fn a_provider_that_is_not_usable_is_never_started() {
        let mut r = rig(&[Claude]);
        for task in TaskType::ALL {
            r.turn(LONG, new_task(task)).await;
        }
        let calls = r.factory_calls.lock().unwrap().clone();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|p| *p == Claude), "{calls:?}");

        // A cooling provider is as unusable as one that is not installed.
        let mut r = rig(&[Claude, Codex]);
        r.engine.runtime.cool_down(Claude, None, NOW);
        for task in TaskType::ALL {
            r.turn(LONG, new_task(task)).await;
        }
        let calls = r.factory_calls.lock().unwrap().clone();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|p| *p == Codex), "{calls:?}");
    }

    #[tokio::test]
    async fn catch_up_is_prepended_on_a_switch_but_not_when_staying_and_is_not_recorded() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        let b = if a == Claude { Codex } else { Claude };
        r.turn("first request", want(TaskType::Feature)).await;
        assert_eq!(r.started().last().unwrap().prompt, "first request", "staying on a fresh provider: no catch-up");

        r.script(a, QUOTA);
        r.turn("second request", want(TaskType::Feature)).await;
        let on_b = r.started().last().unwrap().clone();
        assert_eq!(on_b.provider, b);
        assert!(on_b.prompt.contains("first request") && on_b.prompt.contains("M src/a.rs"), "{}", on_b.prompt);
        assert!(on_b.prompt.ends_with("Now the user says:\nsecond request"), "{}", on_b.prompt);
        let recent = r.engine.sessions.recent_context(1, 200);
        assert!(recent.starts_with("user: second request"), "{recent}");

        r.turn("third request", want(TaskType::Feature)).await;
        assert_eq!(r.started().last().unwrap().prompt, "third request", "B is up to date: no catch-up");
    }

    #[tokio::test]
    async fn session_ids_are_resumed_per_provider() {
        let mut r = rig(&[Claude]);
        r.turn("one", want(TaskType::Feature)).await;
        r.turn("two", want(TaskType::Feature)).await;
        let s = r.started();
        assert_eq!(s[0].session, None);
        assert_eq!(s[1].session.as_deref(), Some("sess-claude"));
    }

    #[tokio::test]
    async fn nothing_usable_says_so_and_starts_nothing() {
        let mut r = rig(&[]);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::NoRoute);
        assert!(r.factory_calls.lock().unwrap().is_empty());
        assert!(report.message.contains("/providers") && report.message.contains("feature"), "{}", report.message);
        assert_eq!(r.ui.notes.len(), 1);
        assert!(r.ui.routes.is_empty());
    }

    #[tokio::test]
    async fn security_never_falls_back_to_deepseek() {
        let mut r = rig(&[DeepSeek]);
        let report = r.turn("audit the login flow", want(TaskType::Security)).await;
        assert_eq!(report.status, TurnStatus::NoRoute);
        assert!(report.message.contains("Security work never goes to deepseek"), "{}", report.message);
        assert!(r.factory_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_pin_that_becomes_unusable_is_reported_not_swapped() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.engine.set_pin(Pin::Provider(a));
        r.script(a, QUOTA);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::NoRoute);
        assert_eq!(r.started().len(), 1);
        assert!(report.message.contains("pin"), "{}", report.message);
    }

    #[tokio::test]
    async fn at_most_six_models_are_tried() {
        let all = [Claude, DeepSeek, Codex, Gemini, Grok];
        let mut r = rig(&all);
        r.engine.runtime.allow = Allowances { credits: true, per_token: true };
        for p in all {
            r.script(p, EMPTY);
        }
        // A very hard debug turn has seven models in its chain: more than the cap.
        let hard = ProviderWant { very_hard_p: 0.95, ..want(TaskType::HardDebug) };
        let report = r.turn("find why the build hangs", hard).await;
        assert_eq!(report.status, TurnStatus::Failed);
        assert_eq!(report.attempts, MAX_ATTEMPTS);
        assert_eq!(r.started().len(), MAX_ATTEMPTS as usize);
        assert!(report.message.contains("stopped after 6 attempts"), "{}", report.message);
        assert!(report.text.is_empty());
    }

    #[tokio::test]
    async fn routing_off_is_claude_only_with_one_attempt() {
        let mut r = rig(&[Codex]);
        r.engine.set_enabled(false);
        let strong = CodeWant { tier: 2, confident: true, reason: "confidently reasoning".into(), signals: Value::Null };
        let report = r.engine.run_judged("why is this slow", Judged::Legacy(strong), &mut r.ui).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!((report.provider, report.model.as_deref()), (Some(Claude), Some(MODEL_STRONG)));
        assert!(r.factory_calls.lock().unwrap().iter().all(|p| *p == Claude));
        assert!(r.ui.routes[0].contains("claude") && r.ui.routes[0].contains(MODEL_STRONG));

        // An easier turn holds Opus (warm cache) instead of dropping at once.
        let easy = CodeWant { tier: 1, confident: true, reason: "everyday".into(), signals: Value::Null };
        let report = r.engine.run_judged("thanks", Judged::Legacy(easy), &mut r.ui).await;
        assert_eq!(report.model.as_deref(), Some(MODEL_STRONG));

        // A failure is reported; there is no failover in this mode.
        r.script(Claude, QUOTA);
        let again = CodeWant { tier: 1, confident: true, reason: "everyday".into(), signals: Value::Null };
        let before = r.started().len();
        let report = r.engine.run_judged("and now", Judged::Legacy(again), &mut r.ui).await;
        assert_eq!(report.status, TurnStatus::Failed);
        assert_eq!(r.started().len(), before + 1);
    }

    #[tokio::test]
    async fn routing_off_honours_a_claude_pin_and_ignores_another_providers() {
        let mut r = rig(&[Claude]);
        let w = || CodeWant { tier: 2, confident: true, reason: "r".into(), signals: Value::Null };
        r.engine.set_pin(Pin::parse("sonnet").unwrap());
        let report = r.engine.run_judged("x", Judged::Legacy(w()), &mut r.ui).await;
        assert_eq!(report.model.as_deref(), Some(MODEL_MID));
        r.engine.set_pin(Pin::Provider(Codex));
        let report = r.engine.run_judged("y", Judged::Legacy(w()), &mut r.ui).await;
        assert_eq!(report.provider, Some(Claude));
        assert!(r.ui.routes.last().unwrap().contains("ignored"));
    }

    #[tokio::test]
    async fn a_missing_jev_judgment_still_routes_to_a_usable_model() {
        let mut r = rig(&[Claude]);
        let report = r.engine.run_judged("hello", Judged::Providers(None), &mut r.ui).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(report.provider, Some(Claude));
    }

    #[tokio::test]
    async fn successful_turns_append_log_rows_with_attempt_and_failover_notes() {
        let path = std::env::temp_dir().join(format!("jev-turn-test-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        r.engine = r.engine.with_turn_log(Some(path.to_string_lossy().into_owned()));
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, QUOTA);
        r.turn("add a retry to the client", want(TaskType::Feature)).await;
        let rows: Vec<Value> =
            std::fs::read_to_string(&path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let _ = std::fs::remove_file(&path);
        assert_eq!(rows.len(), 2, "one row per attempt");
        assert_eq!((rows[0]["ok"].clone(), rows[0]["error_kind"].clone(), rows[0]["attempt"].clone()), (json!(false), json!("quota"), json!(1)));
        assert_eq!((rows[1]["ok"].clone(), rows[1]["attempt"].clone()), (json!(true), json!(2)));
        assert_eq!(rows[1]["signals"]["task"], "feature");
        assert!(rows[1]["reason"].as_str().unwrap().contains("task:"));
        assert!(rows[1]["failover_notes"][0].as_str().unwrap().contains("hit its limit"));
    }

    #[tokio::test]
    async fn new_session_forgets_the_conversation_but_not_the_pin_or_cooldowns() {
        let mut r = rig(&[Claude]);
        r.turn("one", want(TaskType::Feature)).await;
        r.engine.set_pin(Pin::Provider(Claude));
        r.engine.runtime.cool_down(Codex, None, NOW);
        r.engine.new_session();
        assert!(r.engine.sessions.recent_context(3, 100).is_empty());
        assert!(r.engine.routing.current.is_none());
        assert_eq!(r.engine.pin(), &Pin::Provider(Claude));
        assert!(r.engine.runtime.cooldowns.active(Codex, NOW).is_some());
        r.turn("two", want(TaskType::Feature)).await;
        assert_eq!(r.started().last().unwrap().session, None, "a fresh session resumes nothing");
    }

    #[tokio::test]
    async fn the_answer_is_shown_once_even_when_the_cli_only_sends_it_at_the_end() {
        let mut r = rig(&[Claude]);
        r.turn("one", want(TaskType::Feature)).await;
        assert_eq!(r.ui.texts, vec!["the answer".to_string()]);
    }

    #[tokio::test]
    async fn time_comes_from_the_injected_clock() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, QUOTA);
        r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert!(r.engine.runtime.cooldowns.active(a, NOW + 1).is_some());
        // Past the longest possible cooldown the provider is back, with no real waiting.
        r.clock.store(NOW + 9 * 24 * 3_600_000, Ordering::SeqCst);
        r.script(a, OK);
        let report = r.turn(LONG, new_task(TaskType::Feature)).await;
        assert_eq!(report.provider, Some(a));
    }

    #[test]
    fn legacy_tiers_are_catalog_models() {
        for tier in 0..LEGACY_TIERS.len() {
            assert_eq!(legacy_spec(tier).id, LEGACY_TIERS[tier]);
        }
        assert_eq!(legacy_spec(9).id, MODEL_STRONG);
    }

    #[test]
    fn signals_copy_every_field_of_the_want() {
        let mut w = want(TaskType::Docs);
        w.correction_p = Some(0.4);
        w.security_keywords = Security::Weak;
        let s = Signals::from(&w);
        assert_eq!((s.task, s.correction_p, s.security_keywords), (TaskType::Docs, Some(0.4), Security::Weak));
        assert_eq!((s.task_confidence, s.continues_p, s.edit_p), (0.9, 0.9, 0.3));
    }

    #[test]
    fn git_status_short_is_empty_outside_a_repo() {
        assert_eq!(git_status_short(Path::new("/definitely/not/a/dir")), "");
    }

    // ------------------------------------------------------------ orchestration

    /// A multi-part request: Jev's `multi_part` is above the cutoff.
    fn multi(task: TaskType) -> ProviderWant {
        ProviderWant { multi_part_p: 0.9, continues_p: 0.1, ..want(task) }
    }

    fn orchestrating(mut r: Rig) -> Rig {
        let setup = Setup { bin: Some(PathBuf::from("/bin/delegate_mcp")), config_dir: std::env::temp_dir() };
        r.engine = r.engine.with_orchestrate(true).with_orch_setup(Some(setup));
        r
    }

    #[tokio::test]
    async fn orchestration_is_off_by_default_even_for_a_multi_part_top_route() {
        let mut r = rig(&[Claude, Codex]);
        assert!(!r.engine.orchestrate());
        let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert!(r.attached.lock().unwrap().is_empty());
        assert!(r.ui.routes.iter().all(|l| !l.contains("orchestrat")), "{:?}", r.ui.routes);
        assert!(!r.started()[0].prompt.contains("orchestration"));
    }

    #[tokio::test]
    async fn a_multi_part_top_route_runs_on_a_planner_with_the_delegate_tool() {
        let mut r = orchestrating(rig(&[Claude, Codex]));
        let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        let started = r.started();
        let top = started[0].provider;
        assert!(matches!(top, Claude | Codex));
        assert!(find(top, &started[0].model).is_some_and(|m| m.tier == catalog::Tier::Top), "{}", started[0].model);
        let attached = r.attached.lock().unwrap().clone();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].0, top);
        assert_eq!(attached[0].1.name, "jev");
        assert!(started[0].prompt.contains("`delegate` tool") && started[0].prompt.contains("rebuild the whole importer"));
        assert!(r.ui.routes[0].contains("orchestrating") && r.ui.routes[0].contains("Jev picks the workers"), "{:?}", r.ui.routes);
    }

    #[tokio::test]
    async fn orchestration_needs_both_multi_part_and_a_top_route() {
        let mut r = orchestrating(rig(&[Claude, Codex]));
        // Multi-part, but a Feature turn normally goes to an Everyday model.
        r.turn("add the endpoint, the page and the docs", multi(TaskType::Feature)).await;
        // A Top route, but not multi-part.
        r.turn("why does this deadlock", ProviderWant { continues_p: 0.1, ..want(TaskType::Review) }).await;
        assert!(r.attached.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_planner_is_never_a_provider_without_mcp_support() {
        // Review's first choice is Codex, then Gemini; with Codex missing the normal route is Gemini Argon, which cannot take the tool.
        let mut r = orchestrating(rig(&[Gemini, Claude]));
        let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(r.started()[0].provider, Claude, "the planner is the best Top model that can take the tool");
        assert_eq!(r.attached.lock().unwrap()[0].0, Claude);
        assert!(r.ui.routes[0].contains("gemini cannot take the delegate tool"), "{:?}", r.ui.routes);
    }

    #[tokio::test]
    async fn with_no_planner_available_the_turn_runs_normally_and_says_why() {
        let mut r = orchestrating(rig(&[Gemini]));
        let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(r.started()[0].provider, Gemini);
        assert!(r.attached.lock().unwrap().is_empty());
        assert!(r.ui.routes[0].contains("orchestration skipped") && r.ui.routes[0].contains("cannot take the delegate tool"), "{:?}", r.ui.routes);
    }

    #[tokio::test]
    async fn a_missing_delegate_binary_skips_orchestration_with_a_reason() {
        let mut r = rig(&[Claude, Codex]);
        r.engine = r.engine.with_orchestrate(true).with_orch_setup(None);
        r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert!(r.attached.lock().unwrap().is_empty());
        assert!(r.ui.routes[0].contains("orchestration skipped") && r.ui.routes[0].contains("delegate_mcp"), "{:?}", r.ui.routes);
    }

    #[tokio::test]
    async fn a_pinned_provider_without_mcp_support_is_not_overridden_by_orchestration() {
        let mut r = orchestrating(rig(&[Gemini, Claude]));
        r.engine.set_pin(Pin::Provider(Gemini));
        r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        assert_eq!(r.started()[0].provider, Gemini, "a pin is never swapped");
        assert!(r.attached.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_mcp_config_file_for_claude_code_is_written_before_the_planner_starts() {
        let dir = std::env::temp_dir().join(format!("jev-turn-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = rig(&[Claude]);
        let setup = Setup { bin: Some(PathBuf::from("/bin/delegate_mcp")), config_dir: dir.clone() };
        r.engine = r.engine.with_orchestrate(true).with_orch_setup(Some(setup));
        r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
        let path = r.attached.lock().unwrap()[0].1.config_path.clone();
        assert!(path.starts_with(&dir));
        let json: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["mcpServers"]["jev"]["command"], "/bin/delegate_mcp");
        std::fs::remove_dir_all(&dir).ok();
    }
}
