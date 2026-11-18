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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::watch;

use crate::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::backend::{
    self, Approver, Backend, Decision, ErrorKind, Event, McpAttachment, PermissionRequest, ProviderId, TurnError, TurnOutcome,
    TurnRequest, Usage,
};
use crate::cache::{self, WarmCache};
use crate::catalog::{self, find, ModelSpec};
use crate::effort::EffortSetting;
use crate::changes::{self, ChangeSummary, Snapshot, TurnActivity, UndoError, UndoReport};
use crate::guard::{Guards, TurnFacts};
use crate::handoff::{self, ResumeError, SavedSession, Sessions, TurnRecord};
use crate::log;
use crate::mode::Mode;
use crate::orchestrate::{self, Outcome, Setup};
use crate::permissions::{self, Offer, Places, Settled};
use crate::providers::{self, Runtime};
use crate::router::{CodeWant, ProviderWant, Router, Sticky};
use crate::routing::{self, NoRoute, Pin, RouteDecision, RouteInput, RoutingState, Signals};
use crate::turnlog::turn_row;
use crate::uifmt::{self, RouteFacts};

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

    // The structured callbacks below carry the facts behind a line, so a UI can word and
    // style it itself. Each one defaults to the plain-text callback above, so a UI that
    // only prints strings keeps working unchanged.

    /// The routing decision with its parts. Called instead of [`Ui::route`] by the engine.
    fn route_facts(&mut self, facts: &RouteFacts) {
        self.route(&facts.reason);
    }

    /// An attempt failed: either the turn moves on to the next model
    /// ([`Failover::will_retry`]) or this is the final error.
    fn failover(&mut self, f: &Failover) {
        self.note(f.text);
    }

    /// No model can take the turn; nothing was started. `text` is the plain-words message.
    fn no_route(&mut self, _info: &NoRouteInfo, text: &str) {
        self.note(text);
    }

    /// The user stopped the turn.
    fn cancelled(&mut self, text: &str) {
        self.note(text);
    }

    /// A tool call waits for the person's answer. `offer` is what [`Decision::AllowAlways`]
    /// would save, or why "always" is not offered: the engine decided it, the UI only shows it
    /// (and offers the choice only when it is `Ok`). The question is put on screen by this call; the returned [`Answer`] resolves with what the person said
    /// (`None` means they interrupted with Ctrl-C, which stops the turn). Dropping the
    /// answer before it resolves abandons the question (the CLI withdrew it): the UI stops
    /// listening for a key at once, and [`Ui::permission_withdrawn`] follows. A UI that
    /// cannot ask keeps this default, so a run with no terminal denies at once and never
    /// waits for an answer nobody gives.
    fn ask_permission(&mut self, _req: &PermissionRequest, _offer: &Offer) -> Answer {
        Box::pin(std::future::ready(Some(Decision::Deny { reason: backend::NO_ONE_TO_ASK.to_string() })))
    }

    /// How the question for `req` ended, once the engine has saved what "always" saves (or
    /// failed to): the answer line. Called once per answered question, after the save, so
    /// the screen only ever says what is true. A UI without an answer line still hears about
    /// a save that failed.
    fn permission_settled(&mut self, _req: &PermissionRequest, settled: &Settled) {
        if let Settled::NotSaved(e) = settled {
            self.note(&format!("[permissions] {e}; allowed this once, nothing was saved"));
        }
    }

    /// The question asked for `req` was dropped unanswered: take it off the screen. Called
    /// once per such question, never for one that was answered.
    fn permission_withdrawn(&mut self, _req: &PermissionRequest) {}
}

/// The person's answer to a question, not yet given. It owns what it needs, so the UI is
/// free to print while the person thinks.
pub type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = Option<Decision>> + Send>>;

/// The UI as the event callback and the approver share it; they never run at once.
type SharedUi<'a> = Mutex<&'a mut (dyn Ui + 'a)>;

fn lock_ui<'g, 'a>(ui: &'g SharedUi<'a>) -> std::sync::MutexGuard<'g, &'a mut (dyn Ui + 'a)> {
    ui.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Settle a question's answer: the one place "always" is saved. The offer is what was shown;
/// "always" saves exactly that (rule and folder in one write) or, when the file cannot be
/// written, allows once and says so. An "always" with nothing on offer is a plain yes.
pub fn settle(answer: Option<Decision>, offer: &Offer, project: &Path) -> Settled {
    match answer {
        None => Settled::Stopped,
        Some(Decision::Allow) => Settled::Allowed,
        Some(Decision::AllowAlways) => match offer {
            Ok(always) => match permissions::save_always(project, always) {
                Ok(_) => Settled::Remembered(always.clone()),
                Err(e) => Settled::NotSaved(e),
            },
            Err(_) => Settled::Allowed,
        },
        Some(Decision::Deny { reason }) => Settled::Denied(reason),
    }
}

/// What the CLI is told for a settled question; `None` when the person stopped the turn.
pub fn settled_decision(settled: &Settled) -> Option<Decision> {
    match settled {
        Settled::Allowed | Settled::NotSaved(_) => Some(Decision::Allow),
        Settled::Remembered(_) => Some(Decision::AllowAlways),
        Settled::Denied(reason) => Some(Decision::Deny { reason: reason.clone() }),
        Settled::Stopped => None,
    }
}

/// Answers a running turn's tool approvals through the [`Ui`]: decides what "always" would
/// save, asks, saves it when the person says "always", tells the UI how it ended, and stops
/// the turn when they interrupt the question.
struct EngineApprover<'a> {
    ui: &'a SharedUi<'a>,
    cancel: &'a Cancel,
    /// The project folder, where "always" saves its rule.
    dir: &'a Path,
    /// The turn's mode: [`Mode::Auto`] allows tool requests without asking.
    mode: Mode,
}

impl EngineApprover<'_> {
    async fn answer(&self, req: &PermissionRequest) -> Decision {
        let places = Places::from_env(self.dir);
        // Every rule this call needs was saved before ("always"): not asked again, in any mode
        // that asks at all. Plan mode never reaches here for writes (Claude Code refuses them).
        if permissions::remembered(&req.tool, &req.input, req.blocked_path.as_deref(), &places).is_some() {
            lock_ui(self.ui).event(&Event::Hint(uifmt::remembered_allowed(&req.summary)));
            return Decision::Allow;
        }
        if self.mode == Mode::Auto {
            lock_ui(self.ui).event(&Event::Hint(uifmt::auto_allowed(&req.summary)));
            return Decision::Allow;
        }
        let offer = permissions::always_plan(&req.tool, &req.input, req.blocked_path.as_deref(), &places);
        // The question goes up under the lock; the wait for the person does not hold it, so
        // the turn's own events can still be shown, and dropping this future ends the wait.
        let question = lock_ui(self.ui).ask_permission(req, &offer);
        let asked = question.await;
        // A UI that cannot ask: nothing was answered, so there is nothing to settle.
        if let Some(Decision::Deny { reason }) = &asked {
            if reason == backend::NO_ONE_TO_ASK {
                lock_ui(self.ui).event(&Event::Hint(uifmt::denied_without_asking(&req.tool)));
                return Decision::Deny { reason: reason.clone() };
            }
        }
        let settled = settle(asked, &offer, self.dir);
        lock_ui(self.ui).permission_settled(req, &settled);
        settled_decision(&settled).unwrap_or_else(|| {
            self.cancel.cancel();
            Decision::Deny { reason: "the user interrupted the turn".into() }
        })
    }
}

impl Approver for EngineApprover<'_> {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = Decision> + Send + 'a>> {
        Box::pin(self.answer(req))
    }

    fn withdrawn(&self, req: &PermissionRequest) {
        lock_ui(self.ui).permission_withdrawn(req);
    }
}

/// A failed attempt, as the UI is told about it.
#[derive(Debug, Clone, Copy)]
pub struct Failover<'a> {
    pub provider: ProviderId,
    pub model: &'a str,
    pub kind: &'a ErrorKind,
    /// What the CLI said.
    pub message: &'a str,
    /// The failed attempt had already produced text, which was dropped.
    pub discarded: bool,
    /// True: another model is tried next. False: the turn ends here with this error.
    pub will_retry: bool,
    /// The engine's own one-line wording, for UIs that only print text.
    pub text: &'a str,
}

/// Why nothing could be started, as the UI is told about it.
#[derive(Debug, Clone, Copy)]
pub enum NoRouteInfo<'a> {
    /// A `/model` pin names a model that cannot run now.
    Pin { what: &'a str },
    /// No usable model fits this kind of task.
    Nothing { task: &'a str },
    /// Your privacy policy, budget cap or routing rules left no model.
    Refused { why: &'a str },
}

/// A switch that stops one turn without stopping the program (Ctrl-C in the UI). Cheap
/// to clone; every clone sees the same state. Once cancelled it stays cancelled, so make
/// a new one for each turn.
#[derive(Clone, Debug)]
pub struct Cancel {
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl Cancel {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx: Arc::new(tx), rx }
    }

    /// Ask the turn to stop. Safe to call more than once, from any thread.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once [`Cancel::cancel`] has been called (at once if it already was).
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        if rx.wait_for(|c| *c).await.is_err() {
            // Unreachable while `self` holds the sender; never report a cancel that did not happen.
            std::future::pending::<()>().await;
        }
    }
}

/// Resolves when `cancel` fires, or never when `honor` is false.
async fn stop_signal(cancel: &Cancel, honor: bool) {
    if honor {
        cancel.cancelled().await;
    } else {
        std::future::pending::<()>().await;
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// A model answered; see [`TurnReport::text`].
    Answered,
    /// No usable model could take the turn, so nothing was started.
    NoRoute,
    /// A model was started and failed in a way that ended the turn.
    Failed,
    /// The user stopped the turn. Nothing was retried, routing and cooldowns are
    /// unchanged, and the next turn is told this one was interrupted.
    Cancelled,
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
    /// What the turn changed in the working tree. `None` when the folder is not a git
    /// repository, git is missing or failed, no model was started, or the turn changed
    /// nothing (a summary kept here always lists at least one file).
    pub changes: Option<ChangeSummary>,
}

impl TurnReport {
    fn ended(status: TurnStatus, message: String, attempts: u32) -> Self {
        Self { status, text: String::new(), message, provider: None, model: None, usage: None, attempts, changes: None }
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
    idle_timeout(crate::envfile::lookup("CODE_IDLE_TIMEOUT_SECS").as_deref())
}

/// The pure half of [`idle_timeout_from_env`]: seconds as text; anything that is not a
/// positive number leaves the default.
pub fn idle_timeout(value: Option<&str>) -> Duration {
    let secs = value.and_then(|s| s.trim().parse::<u64>().ok()).filter(|s| *s > 0).unwrap_or(900);
    Duration::from_secs(secs)
}

/// The model chosen for one attempt.
struct Chosen {
    spec: &'static ModelSpec,
    reason: String,
    /// The same decision with its parts, for [`Ui::route_facts`].
    facts: RouteFacts,
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
    /// The attempt failed after it had produced text, which was therefore never shown.
    discarded: bool,
    /// The user cancelled it; `result` then holds a placeholder error that is never acted on.
    cancelled: bool,
}

pub struct Engine {
    pub runtime: Runtime,
    pub sessions: Sessions,
    pub routing: RoutingState,
    /// Privacy policy, routing rules, budget, failover policy and the session's meters.
    pub guards: Guards,
    router: Router,
    pin: Pin,
    dir: PathBuf,
    enabled: bool,
    backend_factory: BackendFactory,
    git_status: GitStatus,
    now: Clock,
    idle_timeout: Duration,
    /// Show assistant text as it arrives instead of holding it until the attempt succeeds
    /// (the interactive screen: a failed attempt's text is then followed by a note saying it
    /// was cut off, which the failover line already says).
    stream_text: bool,
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
    /// Snapshot the working tree around every turn (see [`crate::changes`]).
    track_changes: bool,
    /// What the last turn that started a model changed; consumed by an undo.
    last_changes: Option<ChangeSummary>,
    /// Every turn of this session that changed files, oldest first: the checkpoints that
    /// `/diff --turn`, `/diff --all` and `/rewind` work on.
    change_history: Vec<ChangeSummary>,
    /// Images for the next turn (dropped or pasted paths, see [`crate::attach`]); sent with
    /// every attempt of that turn, then dropped.
    images: Vec<PathBuf>,
    /// What the tools of the turn in progress did (all attempts), to tell its changes from
    /// other processes'.
    turn_activity: TurnActivity,
    /// Where saved sessions live (the state directory); `None` means nothing is saved.
    persist: Option<PathBuf>,
    /// The file this session saves to: chosen at its first save, or the one `/resume` took
    /// over. `/new` clears it, so the next save starts another file and the old one stays.
    session_file: Option<PathBuf>,
    /// What a turn may do without asking (`/mode`, Shift-Tab).
    mode: Mode,
    /// `/effort`: the reasoning effort passed to CLIs that have the control.
    effort: EffortSetting,
}

impl Engine {
    /// Real backends, the real clock and git, and the turn log from `JEV_CODE_LOG`.
    /// Routing is on.
    pub fn new(runtime: Runtime, router: Router, dir: PathBuf) -> Self {
        Self {
            runtime,
            sessions: Sessions::new(),
            routing: RoutingState::default(),
            guards: Guards::default(),
            router,
            pin: Pin::Auto,
            dir,
            enabled: true,
            backend_factory: Box::new(providers::session_backend_for),
            git_status: Box::new(git_status_short),
            now: Box::new(log::now_ms),
            idle_timeout: idle_timeout_from_env(),
            stream_text: false,
            turn_log: log::path_from_env("JEV_CODE_LOG", "code-turns.jsonl"),
            previous: None,
            last_context_tokens: 0,
            sticky: Sticky::default(),
            downgrade_after: 2,
            orchestrate: false,
            orch_setup: None,
            track_changes: true,
            last_changes: None,
            change_history: Vec::new(),
            images: Vec::new(),
            turn_activity: TurnActivity::default(),
            persist: None,
            session_file: None,
            mode: Mode::default(),
            effort: EffortSetting::default(),
        }
    }

    /// Keep a summary of what each turn changed, for `/diff` and `/undo` (on by default;
    /// it costs two `git` runs per turn and does nothing outside a repository).
    pub fn with_track_changes(mut self, on: bool) -> Self {
        self.track_changes = on;
        self
    }

    /// Save the session after every turn under `state_dir/sessions/` so it can be resumed
    /// after a restart. `None` (the default of the library) saves nothing; the binary
    /// passes [`log::state_dir`].
    pub fn with_persistence(mut self, state_dir: Option<PathBuf>) -> Self {
        // The settings that fence in the router live beside the saved sessions.
        self.guards = Guards::load(state_dir.as_deref(), &self.dir);
        self.persist = state_dir;
        self
    }

    /// Use these guards instead of the ones read from the settings files.
    pub fn with_guards(mut self, guards: Guards) -> Self {
        self.guards = guards;
        self
    }

    /// Run `/budget`, `/privacy`, `/failover` or `/health` against this session's guards.
    pub fn run_guard_command(&mut self, cmd: crate::guard_cmd::GuardCmd) -> String {
        let plans: Vec<(String, f64)> = crate::providers::ALL_PROVIDERS
            .into_iter()
            .filter_map(|p| self.runtime.plan_utilization(p).map(|u| (p.to_string(), u)))
            .collect();
        crate::guard_cmd::run(cmd, &mut self.guards, self.persist.as_deref(), &plans)
    }

    /// Show assistant text the moment it arrives (see the field).
    pub fn with_streaming(mut self) -> Self {
        self.stream_text = true;
        self
    }

    /// Start in this mode (the binary reads `CODE_PERMISSION_MODE`).
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
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

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Applies from the next turn on; a turn already running keeps the mode it started in.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn effort(&self) -> EffortSetting {
        self.effort
    }

    /// Applies from the next turn on.
    pub fn set_effort(&mut self, effort: EffortSetting) {
        self.effort = effort;
    }

    /// The prefix the last answering provider has cached, for `/cache` and the footer.
    pub fn warm_cache(&self) -> Option<WarmCache> {
        self.routing.warm
    }

    pub fn orchestrate(&self) -> bool {
        self.orchestrate
    }

    pub fn set_orchestrate(&mut self, on: bool) {
        self.orchestrate = on;
    }

    /// Images to send with the next turn (and only that one).
    pub fn attach_images(&mut self, images: Vec<PathBuf>) {
        self.images = images;
    }

    /// `/new`: forget the conversation and every provider session. The pin, the
    /// routing switch and what is known about providers stay.
    pub fn new_session(&mut self) {
        self.sessions.reset();
        self.routing = RoutingState::default();
        self.sticky = Sticky::default();
        self.previous = None;
        self.last_context_tokens = 0;
        self.last_changes = None;
        self.change_history.clear();
        self.runtime.forget_unusable_here();
        // The saved file stays (it can be resumed); the next save starts a new one.
        self.session_file = None;
    }

    // ------------------------------------------------------------------ changes

    /// What the last turn changed, if it changed anything in a git working tree.
    pub fn last_changes(&self) -> Option<&ChangeSummary> {
        self.last_changes.as_ref()
    }

    /// The turns that changed files, oldest first.
    pub fn change_history(&self) -> &[ChangeSummary] {
        &self.change_history
    }

    /// Put the working tree back to how it was before checkpoint `number` (1-based, as
    /// `/diff --turn` counts), undoing the newer checkpoints first. Stops at the first
    /// one that cannot be undone and reports how far it got.
    pub fn rewind_to(&mut self, number: usize) -> Result<Vec<UndoReport>, (Vec<UndoReport>, UndoError)> {
        if number == 0 || number > self.change_history.len() {
            return Err((Vec::new(), UndoError::NothingToUndo));
        }
        let mut done = Vec::new();
        while self.change_history.len() >= number {
            // The newest checkpoint is the one `undo_last` works on.
            self.last_changes = self.change_history.last().cloned();
            match self.undo_last() {
                Ok(r) => done.push(r),
                Err(e) => return Err((done, e)),
            }
        }
        self.last_changes = self.change_history.last().cloned();
        Ok(done)
    }

    /// The user changed the working tree on purpose (a hunk reverted in `/diff --review`):
    /// record the tree as it is now as where the newest checkpoint ended, so `/undo` and
    /// `/rewind` keep working instead of reading the change as someone else's edit.
    /// Returns false when there is no checkpoint or the tree cannot be read.
    pub fn adopt_working_tree(&mut self) -> bool {
        let Some(now) = changes::snapshot(&self.dir) else { return false };
        let Some(newest) = self.change_history.last_mut() else { return false };
        newest.after = now.tree.clone();
        // Untracked files the turn added: follow their new look, drop the ones now gone.
        newest.added_stamps = newest.added_stamps.keys().filter_map(|k| now.untracked.get(k).map(|s| (k.clone(), *s))).collect();
        if self.last_changes.is_some() {
            self.last_changes = Some(newest.clone());
        }
        self.save_quietly();
        true
    }

    /// The text diff of the last turn, whole or for one repository-relative path; plain
    /// text, for the UI to colour. `None` when there is no summary or git fails.
    pub fn last_diff(&self, path: Option<&str>) -> Option<String> {
        let c = self.last_changes.as_ref()?;
        changes::diff_text(&self.dir, c, path)
    }

    /// Put back what the last turn changed, and only that, if nothing it touched was
    /// edited since (see [`changes::undo`]). One level: the summary is consumed on
    /// success. The conversation is told, so the next catch-up is truthful. Blocking
    /// (a few git runs), like any user command.
    pub fn undo_last(&mut self) -> Result<UndoReport, UndoError> {
        let summary = self.last_changes.clone().ok_or(UndoError::NothingToUndo)?;
        let report = changes::undo(&self.dir, &summary)?;
        self.last_changes = None;
        self.change_history.pop();
        self.sessions.record_undo();
        self.save_quietly();
        Ok(report)
    }

    // ------------------------------------------------------------------ saved sessions

    /// The file this session saves to; `None` until its first save, when persistence is
    /// off, and after `/new`.
    pub fn session_path(&self) -> Option<&Path> {
        self.session_file.as_deref()
    }

    /// Save the session now and say why not on failure. A session with no turns is not
    /// written. Each session has its own file (see [`handoff::new_session_file`]), so
    /// saving never touches an earlier session; the oldest beyond ten are deleted.
    /// Turns save themselves best-effort; the binary calls this once more on exit.
    pub fn save_now(&mut self) -> std::io::Result<()> {
        let Some(state) = self.persist.clone() else { return Ok(()) };
        if self.sessions.turns() == 0 {
            return Ok(());
        }
        let saved_ms = (self.now)();
        let path = match &self.session_file {
            Some(p) => p.clone(),
            None => handoff::new_session_file(&state, &self.dir, saved_ms),
        };
        let saved = SavedSession {
            project: handoff::project_key(&self.dir),
            saved_ms,
            sessions: self.sessions.clone(),
            current: self.routing.current.clone(),
        };
        handoff::write_atomic(&path, &saved.to_json().to_string())?;
        handoff::prune(&state, &self.dir, &path);
        self.session_file = Some(path);
        Ok(())
    }

    /// A failed save never fails a turn.
    fn save_quietly(&mut self) {
        let _ = self.save_now();
    }

    /// Load the most recent saved session of this folder other than the one running: the
    /// conversation, each provider's CLI session, and where routing stood. Replaces the
    /// current conversation, and from then on saves into the file it came from. Asking
    /// again steps back to the one before.
    pub fn resume(&mut self) -> Result<handoff::SessionInfo, ResumeError> {
        let state = self.persist.as_ref().ok_or(ResumeError::Disabled)?;
        let (path, saved) = handoff::load_latest(state, &self.dir, (self.now)(), self.session_file.as_deref())?;
        Ok(self.adopt(path, saved))
    }

    /// The saved sessions of this folder that `/resume` can pick up, newest first, without
    /// the one that is running.
    pub fn resumable(&self) -> Result<Vec<handoff::SessionEntry>, ResumeError> {
        let state = self.persist.as_ref().ok_or(ResumeError::Disabled)?;
        Ok(handoff::list_sessions(state, &self.dir, (self.now)(), self.session_file.as_deref()))
    }

    /// Resume the session saved in `path`, one of [`Engine::resumable`].
    pub fn resume_from(&mut self, path: &std::path::Path) -> Result<handoff::SessionInfo, ResumeError> {
        let saved = handoff::load_path(&self.dir, path)?;
        Ok(self.adopt(path.to_path_buf(), saved))
    }

    fn adopt(&mut self, path: std::path::PathBuf, saved: handoff::SavedSession) -> handoff::SessionInfo {
        self.session_file = Some(path);
        let info = saved.info();
        self.previous = saved.sessions.last_answered().map(|(p, r)| (p.to_string(), r.to_string()));
        self.sessions = saved.sessions;
        self.routing = RoutingState::default();
        self.routing.current = saved.current;
        self.last_context_tokens = 0;
        self.last_changes = None;
        self.change_history.clear();
        info
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

    /// Run one user turn that cannot be cancelled.
    pub async fn run(&mut self, prompt: &str, ui: &mut dyn Ui) -> TurnReport {
        self.run_cancellable(prompt, ui, &Cancel::new()).await
    }

    /// Run one user turn that `cancel` can stop: the Jev call and the running CLI (with
    /// everything it started) are dropped at once, and the report says
    /// [`TurnStatus::Cancelled`].
    pub async fn run_cancellable(&mut self, prompt: &str, ui: &mut dyn Ui, cancel: &Cancel) -> TurnReport {
        let judged = tokio::select! {
            j = self.judge(prompt, ui) => j,
            _ = cancel.cancelled() => {
                self.images.clear();
                return self.cancelled_report(prompt, None, 0, ui);
            }
        };
        let report = self.run_judged_cancellable(prompt, judged, ui, cancel).await;
        // Attachments belong to the turn they were sent with.
        self.images.clear();
        report
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
        let want = self.router.provider_want(prompt, &recent, previous, self.orchestrate).await;
        if want.is_none() {
            ui.note("[jev] unavailable (timeout or error); routing without its judgment");
        }
        Judged::Providers(want)
    }

    /// Run a turn whose Jev judgment is already in hand: choose, hand off, run, and
    /// fail over. Split from [`Engine::run`] so everything after the network call is
    /// testable with fake backends.
    pub async fn run_judged(&mut self, prompt: &str, judged: Judged, ui: &mut dyn Ui) -> TurnReport {
        self.run_judged_cancellable(prompt, judged, ui, &Cancel::new()).await
    }

    /// [`Engine::run_judged`] that `cancel` can stop. Around the turn proper it takes
    /// the working-tree snapshots (for [`TurnReport::changes`]) and saves the session.
    pub async fn run_judged_cancellable(&mut self, prompt: &str, judged: Judged, ui: &mut dyn Ui, cancel: &Cancel) -> TurnReport {
        let mut before: Option<Snapshot> = None;
        self.turn_activity = TurnActivity::default();
        let mut report = self.run_attempts(prompt, judged, ui, cancel, &mut before).await;
        if report.attempts > 0 {
            // A turn the user cancelled has spent its signal, so its summary is not raced
            // against it; a Ctrl-C while summarising a finished turn still returns at once.
            report.changes = self.changes_since(before, self.turn_activity.clone(), cancel, report.status != TurnStatus::Cancelled).await;
            self.last_changes = report.changes.clone();
            self.change_history.extend(report.changes.clone());
            self.save_quietly();
        }
        report
    }

    /// The working tree as a [`Snapshot`], or `None` (changes are not tracked, not a repo,
    /// git failed, or `cancel` fired while `honor_cancel`; git then finishes in the background).
    async fn snapshot_tree(&self, cancel: &Cancel, honor_cancel: bool) -> Option<Snapshot> {
        if !self.track_changes {
            return None;
        }
        changes::snapshot_until(self.dir.clone(), stop_signal(cancel, honor_cancel)).await
    }

    /// What differs from the `before` snapshot now; `None` if there is no `before`, git
    /// fails, or nothing changed.
    async fn changes_since(&self, before: Option<Snapshot>, activity: TurnActivity, cancel: &Cancel, honor_cancel: bool) -> Option<ChangeSummary> {
        let before = before?;
        let after = self.snapshot_tree(cancel, honor_cancel).await?;
        let summary = changes::summarize_until(self.dir.clone(), before, after, activity, stop_signal(cancel, honor_cancel)).await?;
        (!summary.is_empty()).then_some(summary)
    }

    async fn run_attempts(
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
            let chosen = match self.choose(&judged, signals.as_ref(), prompt, &base, &excluded, attempts, now) {
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
            let attempt = self.run_attempt(chosen, prompt, ui, cancel).await;
            self.turn_activity.add_tools(&attempt.tools);
            self.log_attempt(&attempt, &jev, attempts, &notes);
            if !attempt.cancelled {
                match &attempt.result {
                    Ok(_) => self.guards.scoreboard.record_answer(attempt.chosen.spec.provider, attempt.wall_ms),
                    Err(e) => self.guards.scoreboard.record_failure(attempt.chosen.spec.provider, &e.kind, attempt.wall_ms),
                }
            }
            if attempt.cancelled {
                return self.cancelled_report(prompt, Some(&attempt), attempts, ui);
            }
            let err = match attempt.result.clone() {
                Ok(out) => return self.finish(prompt, attempt, out, attempts),
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
        attempts: u32,
        now: u64,
    ) -> Result<Chosen, NoRoute> {
        if let Judged::Legacy(want) = judged {
            return Ok(self.legacy_choice(want, base));
        }
        let usable = self.runtime.usable(now).without(excluded);
        let turn = TurnFacts {
            root: &self.dir,
            prompt,
            task: signals.map(|s| s.task),
            attached: &self.images,
            pinned: !matches!(self.pin, Pin::Auto),
            attempts,
        };
        let restricted = self.guards.restrict(&usable, &turn).map_err(|why| NoRoute::Refused { why })?;
        if attempts == 0 {
            self.guards.audit_blocked(&restricted.blocked, now);
        }
        let (usable, rule_notes) = (restricted.usable, restricted.notes);
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
            now_ms: now,
        };
        let mut state = base.clone();
        let decision = routing::decide(&mut state, &input)?;
        let outcome = self.plan_orchestration(signals, &decision, &input, base);
        let (spec, reason, mcp) = self.apply_orchestration(outcome, &decision);
        let reason = if rule_notes.is_empty() { reason } else { format!("{reason}; {}", rule_notes.join("; ")) };
        let facts = RouteFacts {
            provider: spec.provider.to_string(),
            model: spec.id.to_string(),
            task: decision.task.to_string(),
            reason: reason.clone(),
            switched_provider: decision.switched_provider,
            dropped_tier: decision.dropped_tier,
        };
        Ok(Chosen { spec, reason, facts, state, legacy_tier: None, mcp })
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
            signals.map(|s| &s.orchestration),
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
        let reason = format!("{} {} — {why}{ignored}", spec.provider, spec.id);
        let facts = RouteFacts {
            provider: spec.provider.to_string(),
            model: spec.id.to_string(),
            reason: reason.clone(),
            ..RouteFacts::default()
        };
        Chosen { spec, reason, facts, state: base.clone(), legacy_tier, mcp: None }
    }

    // ------------------------------------------------------------------ running

    /// The prompt for `provider`: with the catch-up in front when its own session
    /// missed turns that ran elsewhere. This does not mark anything as delivered: the
    /// provider has only seen the catch-up once an attempt ends successfully
    /// ([`Engine::finish`] records the turn, which marks it caught up). An attempt
    /// that fails first leaves the turns missed, so the next one sends them again.
    fn prompt_for(&self, provider: ProviderId, model: &str, prompt: &str) -> String {
        if self.sessions.missed_turns(provider) == 0 {
            return prompt.to_string();
        }
        let git = (self.git_status)(&self.dir);
        match self.sessions.catch_up_for(provider, model, &git, CATCH_UP_TOKENS) {
            Some(text) => format!("{text}{prompt}"),
            None => prompt.to_string(),
        }
    }

    async fn run_attempt(&mut self, chosen: Chosen, prompt: &str, ui: &mut dyn Ui, cancel: &Cancel) -> Attempt {
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
            images: self.images.clone(),
            session_id: self.sessions.session_id(provider).map(String::from),
            mode: self.mode,
        };
        let mut tools: Vec<String> = Vec::new();
        // Progress (tools, plan) is shown as it happens; assistant text is held back until
        // the attempt is known to have succeeded, so a failed attempt's partial commentary
        // never sits on screen as if it were an answer.
        let mut held: Vec<Event> = Vec::new();
        let idle = self.idle_timeout;
        let stream = self.stream_text;
        let started = Instant::now();
        let runtime = &mut self.runtime;
        let result = {
            let shared_ui: SharedUi = Mutex::new(&mut *ui);
            let approver = EngineApprover { ui: &shared_ui, cancel, dir: &self.dir, mode: self.mode };
            let mut on_event = |e: &Event| match e {
                Event::Text(_) => {
                    if stream {
                        lock_ui(&shared_ui).event(e);
                    }
                    held.push(e.clone());
                }
                other => {
                    lock_ui(&shared_ui).event(other);
                    match other {
                        Event::Tool(summary) => tools.push(summary.clone()),
                        Event::Plan(plan) => runtime.note_plan(provider, plan.clone()),
                        _ => {}
                    }
                }
            };
            // Dropping the turn future on a cancel kills the CLI and its children.
            tokio::select! {
                r = backend::run_turn_with(&*backend, &req, idle, &mut on_event, &approver) => Some(r),
                _ = cancel.cancelled() => None,
            }
        };
        let cancelled = result.is_none();
        let result = result.unwrap_or_else(|| Err(TurnError::new(ErrorKind::Other, "cancelled by the user")));
        let discarded = result.is_err() && !held.is_empty();
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
        Attempt { chosen, tools, wall_ms: started.elapsed().as_millis() as u64, result, discarded, cancelled }
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
            changes: None,
        }
    }

    /// End a turn the user stopped. If a model had been started, the turn is remembered
    /// as interrupted so a later catch-up (or the same model's next prompt) knows files
    /// may be half changed. Nothing else moves: no failover, no cooldown, no routing state.
    fn cancelled_report(&mut self, prompt: &str, attempt: Option<&Attempt>, attempts: u32, ui: &mut dyn Ui) -> TurnReport {
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
            ..TurnReport::ended(TurnStatus::Cancelled, message, attempts)
        }
    }

    /// Remember a turn that got no answer, so a later catch-up says it was asked.
    fn record_failed(&mut self, prompt: &str, attempt: &Attempt, err: &TurnError) {
        self.sessions.record_failed(TurnRecord {
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
            if a.cancelled {
                o.insert("error_kind".into(), json!("cancelled"));
                o.insert("cancelled".into(), json!(true));
            }
            if !notes.is_empty() {
                o.insert("failover_notes".into(), json!(notes));
            }
        }
        log::append(path, &row);
    }

    /// Plain words for "no model can take this turn", never a guess at one.
    fn no_route_message(&self, e: &NoRoute, now: u64, excluded: &HashSet<(ProviderId, String)>) -> String {
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
    const UNUSABLE_HERE: &str = "printf 'FAIL:Not inside a trusted directory and --skip-git-repo-check was not specified.\\n'";
    const FAILS_AFTER_TALKING: &str =
        "printf 'TOOL:Read a.rs\\nTEXT:let me try this approach\\nFAIL:usage limit reached|1794853451\\n'";
    const TALKS_THEN_ANSWERS: &str = "printf 'TEXT:first\\nTOOL:Edit b.rs\\nTEXT:second\\nFIN:final\\n'";
    const ANSWER_ALREADY_SAID: &str = "printf 'TEXT:thinking\\nTEXT:done\\nFIN:done\\n'";

    /// A "CLI" that is a shell script, with a grammar of canned lines (see backend tests).
    struct Fake {
        provider: ProviderId,
        script: &'static str,
        started: Arc<Mutex<Vec<Started>>>,
        attached: Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>,
        /// The rig's clock, and how long (ms) an attempt takes before it reports a failure.
        clock: Arc<AtomicU64>,
        attempt_ms: Arc<AtomicU64>,
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
                Some(("FAIL", m)) => {
                    self.clock.fetch_add(self.attempt_ms.load(Ordering::SeqCst), Ordering::SeqCst);
                    Event::Failed { message: m.into() }
                }
                Some(("TOOL", t)) => Event::Tool(t.into()),
                Some(("TEXT", t)) => Event::Text(t.into()),
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
        /// Everything in the order the Ui saw it: "route:", "tool:", "text:", "note:".
        seen: Vec<String>,
        /// Cancel this as soon as a tool call is shown, which is deterministic where a timer is not.
        cancel_on_tool: Option<Cancel>,
        /// Cancel as soon as a note is shown: the failover note, which falls between two attempts.
        cancel_on_note: Option<Cancel>,
    }

    impl Ui for Recorder {
        fn route(&mut self, line: &str) {
            self.routes.push(line.into());
            self.seen.push("route:".into());
        }
        fn event(&mut self, e: &Event) {
            match e {
                Event::Text(t) => {
                    self.texts.push(t.clone());
                    self.seen.push(format!("text:{t}"));
                }
                Event::Tool(t) => {
                    self.seen.push(format!("tool:{t}"));
                    if let Some(c) = &self.cancel_on_tool {
                        c.cancel();
                    }
                }
                _ => {}
            }
        }
        fn note(&mut self, line: &str) {
            self.notes.push(line.into());
            self.seen.push("note:".into());
            if let Some(c) = &self.cancel_on_note {
                c.cancel();
            }
        }
    }

    struct Rig {
        engine: Engine,
        scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>>,
        started: Arc<Mutex<Vec<Started>>>,
        attached: Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>,
        factory_calls: Arc<Mutex<Vec<ProviderId>>>,
        clock: Arc<AtomicU64>,
        /// How long (ms on `clock`) a fake attempt runs before it reports a failure.
        attempt_ms: Arc<AtomicU64>,
        ui: Recorder,
    }

    fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    fn rig_with(states: &[(ProviderId, State)], script: &'static str) -> Rig {
        rig_in(std::env::temp_dir(), states, script)
    }

    /// A rig whose engine works in `dir`. Change tracking is off unless a test turns it on.
    fn rig_in(dir: PathBuf, states: &[(ProviderId, State)], script: &'static str) -> Rig {
        let avail = states.iter().map(|(p, s)| availability(*p, *s)).collect();
        let router = Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0);
        let scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>> = Arc::new(Mutex::new(
            [Claude, Codex, Gemini, Grok, DeepSeek].into_iter().map(|p| (p, script)).collect(),
        ));
        let started = Arc::new(Mutex::new(Vec::new()));
        let attached = Arc::new(Mutex::new(Vec::new()));
        let factory_calls = Arc::new(Mutex::new(Vec::new()));
        let clock = Arc::new(AtomicU64::new(NOW));
        let attempt_ms = Arc::new(AtomicU64::new(0));
        let (s, st, fc, ck, at, fk, am) = (
            scripts.clone(),
            started.clone(),
            factory_calls.clone(),
            clock.clone(),
            attached.clone(),
            clock.clone(),
            attempt_ms.clone(),
        );
        let engine = Engine::new(Runtime::for_tests(avail), router, dir)
            .with_track_changes(false)
            .with_persistence(None)
            .with_backend_factory(Box::new(move |p| {
                fc.lock().unwrap().push(p);
                let script = s.lock().unwrap()[&p];
                Box::new(Fake {
                    provider: p,
                    script,
                    started: st.clone(),
                    attached: at.clone(),
                    clock: fk.clone(),
                    attempt_ms: am.clone(),
                })
            }))
            .with_git_status(Box::new(|_| " M src/a.rs\n".to_string()))
            .with_clock(Box::new(move || ck.load(Ordering::SeqCst)))
            .with_idle_timeout(Duration::from_millis(300))
            .with_turn_log(None);
        Rig { engine, scripts, started, attached, factory_calls, clock, attempt_ms, ui: Recorder::default() }
    }

    fn states_for(ready: &[ProviderId]) -> Vec<(ProviderId, State)> {
        let all = [Claude, DeepSeek, Codex, Gemini, Grok];
        all.iter().map(|p| (*p, if ready.contains(p) { State::Ready } else { State::NotLoggedIn })).collect()
    }

    fn rig(ready: &[ProviderId]) -> Rig {
        rig_with(&states_for(ready), OK)
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
            orchestration: crate::router::Orchestration::default(),
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
        assert_eq!(r.engine.runtime.blocked.get(&a), Some(&providers::BlockReason::Auth));
        assert!(r.ui.notes.iter().any(|n| n.contains("log in to") && n.contains(providers::login_target(a))), "{:?}", r.ui.notes);
    }

    #[tokio::test]
    async fn a_directory_refusal_fails_over_blocks_the_provider_and_new_session_lifts_it() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        let b = if a == Claude { Codex } else { Claude };
        r.script(a, UNUSABLE_HERE);

        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(report.provider, Some(b));
        assert_eq!(report.attempts, 2);
        assert_eq!(r.started().iter().map(|x| x.provider).collect::<Vec<_>>(), vec![a, b], "rerun on the other provider");
        assert_eq!(r.engine.runtime.blocked.get(&a), Some(&providers::BlockReason::UnusableHere));
        assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none(), "not a usage limit");
        assert!(r.ui.notes.iter().any(|n| n.contains("trying the next model") && n.contains(&a.to_string())), "{:?}", r.ui.notes);

        // Blocked for the rest of the session, even after a probe ...
        r.engine.runtime.avail = ready.iter().map(|p| availability(*p, State::Ready)).collect();
        r.turn(LONG, new_task(TaskType::Feature)).await;
        assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 1, "never started again");
        // ... until /new.
        r.engine.new_session();
        assert!(r.engine.runtime.blocked.is_empty());
        r.script(a, OK);
        r.script(b, UNUSABLE_HERE);
        r.turn(LONG, new_task(TaskType::Feature)).await;
        assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 2);
    }

    #[tokio::test]
    async fn a_failed_attempts_text_never_reaches_the_ui_and_the_note_says_so() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, FAILS_AFTER_TALKING);

        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert!(r.ui.texts.iter().all(|t| t == "the answer"), "{:?}", r.ui.texts);
        assert!(r.ui.notes.iter().any(|n| n.contains("(output of the failed attempt discarded)")), "{:?}", r.ui.notes);
        // The failed attempt's tool call was progress and stays visible, ahead of the failover.
        let tool = r.ui.seen.iter().position(|s| s == "tool:Read a.rs").expect("tool shown");
        let note = r.ui.seen.iter().position(|s| s == "note:").expect("failover note");
        assert!(tool < note);
    }

    #[tokio::test]
    async fn a_failed_attempt_without_text_has_nothing_to_say_was_discarded() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, QUOTA);
        r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert!(r.ui.notes.iter().all(|n| !n.contains("discarded")), "{:?}", r.ui.notes);
    }

    #[tokio::test]
    async fn a_successful_attempts_text_arrives_in_order_after_its_tools_which_stream_live() {
        let mut r = rig(&[Claude]);
        r.script(Claude, TALKS_THEN_ANSWERS);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        // The tool came through while the attempt ran, ahead of text that was said before it.
        assert_eq!(
            &r.ui.seen[1..],
            ["tool:Edit b.rs", "text:first", "text:second", "text:final"],
            "{:?}",
            r.ui.seen
        );

        // An answer already among the texts is not shown a second time.
        r.script(Claude, ANSWER_ALREADY_SAID);
        r.ui = Recorder::default();
        r.turn("and add a test", want(TaskType::Feature)).await;
        assert_eq!(r.ui.texts, ["thinking", "done"]);

        r.script(Claude, "printf 'TEXT:first\\nTEXT:second\\nFIN:first second\\n'");
        r.ui = Recorder::default();
        r.turn("and explain", want(TaskType::Explain)).await;
        assert_eq!(r.ui.texts, ["first", "second"]);
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
    async fn a_failed_attempt_leaves_the_catch_up_owed_and_a_success_settles_it() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        let b = if a == Claude { Codex } else { Claude };
        r.turn("first request", new_task(TaskType::Feature)).await;
        assert_eq!(r.started().last().unwrap().provider, a);

        // A stalls on the second turn, so B answers it: A has now missed that turn.
        r.script(a, STALL);
        r.turn("second request", new_task(TaskType::Feature)).await;
        assert_eq!(r.started().last().unwrap().provider, b);

        // A is tried again and stalls again. It was handed the catch-up, but never
        // answered, so it is still owed it.
        r.turn("third request", new_task(TaskType::Feature)).await;
        let failed_try = r.started().into_iter().filter(|s| s.provider == a).last().unwrap();
        assert!(failed_try.prompt.contains("second request"), "{}", failed_try.prompt);
        assert_eq!(r.engine.sessions.missed_turns(a), 2, "the stalled attempt did not count as delivery");

        // Routing stays with B for a while after a switch, so take B out of the turn.
        r.script(a, OK);
        r.script(b, STALL);
        r.turn("fourth request", new_task(TaskType::Feature)).await;
        let on_a = r.started().last().unwrap().clone();
        assert_eq!(on_a.provider, a);
        assert!(on_a.prompt.contains("second request") && on_a.prompt.contains("third request"), "{}", on_a.prompt);
        assert!(on_a.prompt.ends_with("Now the user says:\nfourth request"), "{}", on_a.prompt);

        r.turn("fifth request", new_task(TaskType::Feature)).await;
        let again = r.started().last().unwrap().clone();
        assert_eq!((again.provider, again.prompt.as_str()), (a, "fifth request"), "delivered once, not repeated");
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

    #[tokio::test]
    async fn a_duration_reset_counts_from_when_the_error_arrived_not_from_when_the_attempt_began() {
        const MINUTE: u64 = 60_000;
        const RESETS_IN_3H20: &str = "printf 'FAIL:You have hit your limit, resets in 3h20m\\n'";
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, RESETS_IN_3H20);
        r.attempt_ms.store(10 * MINUTE, Ordering::SeqCst); // the turn ran 10 minutes before failing

        r.turn("add a retry to the client", want(TaskType::Feature)).await;
        let reset = NOW + 10 * MINUTE + 200 * MINUTE;
        assert_eq!(r.engine.runtime.cooldowns.active(a, NOW), Some(reset - NOW));
        assert!(r.engine.runtime.cooldowns.active(a, reset - 1).is_some(), "not over 10 minutes early");
        assert!(r.engine.runtime.cooldowns.active(a, reset).is_none());
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
        ProviderWant { orchestration: crate::router::Orchestration::new(0.9, 0.1), continues_p: 0.1, ..want(task) }
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

    // ------------------------------------------------------------------ cancelling

    /// Talks, then hangs: a turn that only a cancel can end quickly.
    const HANGS: &str = "printf 'TOOL:Read a.rs\\nTEXT:half an answer\\n'; sleep 30";

    #[tokio::test]
    async fn a_cancel_token_is_shared_by_its_clones_and_resolves_once_cancelled() {
        let c = Cancel::new();
        let other = c.clone();
        assert!(!c.is_cancelled());
        assert!(tokio::time::timeout(Duration::from_millis(50), c.cancelled()).await.is_err(), "pending until cancelled");
        other.cancel();
        other.cancel();
        assert!(c.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), c.cancelled()).await.expect("resolves at once when already cancelled");
        assert!(!Cancel::default().is_cancelled());
    }

    #[tokio::test]
    async fn cancelling_a_running_turn_returns_quickly_without_retry_or_an_answer() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, HANGS);
        let cancel = Cancel::new();
        r.ui.cancel_on_tool = Some(cancel.clone());
        let log = std::env::temp_dir().join(format!("jev-cancel-log-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&log);
        r.engine.turn_log = Some(log.to_string_lossy().into_owned());
        let routing_before = r.engine.routing.current.clone();

        let started = Instant::now();
        let report = r
            .engine
            .run_judged_cancellable("add a retry to the client", Judged::Providers(Some(want(TaskType::Feature))), &mut r.ui, &cancel)
            .await;
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(report.status, TurnStatus::Cancelled);
        assert_eq!((report.attempts, report.provider), (1, Some(a)));
        assert!(report.text.is_empty() && report.usage.is_none());
        assert!(report.message.contains("cancelled"));

        // Nothing was retried on the other provider, and nothing else moved.
        assert_eq!(r.started().len(), 1);
        assert_eq!(r.engine.routing.current, routing_before);
        assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
        assert!(r.engine.runtime.unusable_models.is_empty() && r.engine.runtime.blocked.is_empty());
        // Half an answer was never shown as one; the tool progress was.
        assert!(r.ui.texts.is_empty(), "{:?}", r.ui.texts);
        assert!(r.ui.seen.contains(&"tool:Read a.rs".to_string()));
        assert!(r.engine.sessions.session_id(a).is_none(), "a cancelled turn gives no session id");

        // The next catch-up tells the model the turn was cut short.
        let b = if a == Claude { Codex } else { Claude };
        let text = r.engine.sessions.catch_up(b, "", 10_000).expect("recorded");
        assert!(text.contains("add a retry to the client") && text.contains("FAILED"), "{text}");
        assert!(text.contains("interrupted by the user before it finished; files may be partly changed"), "{text}");
        assert!(!text.contains("half an answer"));
        // The turn log row says so.
        let row: Value = serde_json::from_str(std::fs::read_to_string(&log).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!((row["ok"].clone(), row["error_kind"].clone(), row["cancelled"].clone()), (json!(false), json!("cancelled"), json!(true)));
        let _ = std::fs::remove_file(&log);

        // The session goes on: the same model takes the next turn.
        r.ui.cancel_on_tool = None;
        r.script(a, OK);
        let next = r.turn("try again", want(TaskType::Feature)).await;
        assert_eq!((next.status, next.provider), (TurnStatus::Answered, Some(a)));
        assert!(r.started()[1].prompt.contains("try again"));
    }

    #[tokio::test]
    async fn the_provider_that_ran_a_cancelled_turn_is_told_about_it_on_its_next_prompt() {
        let mut r = rig(&[Codex]);
        r.script(Codex, HANGS);
        let cancel = Cancel::new();
        r.ui.cancel_on_tool = Some(cancel.clone());
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        let report = r.engine.run_judged_cancellable("rename the client", judged, &mut r.ui, &cancel).await;
        assert_eq!((report.status, report.provider), (TurnStatus::Cancelled, Some(Codex)));
        assert_eq!(r.engine.sessions.missed_turns(Codex), 1, "its own CLI session never recorded the killed turn");

        r.ui.cancel_on_tool = None;
        r.script(Codex, OK);
        let next = r.turn("try again", want(TaskType::Feature)).await;
        assert_eq!((next.status, next.provider), (TurnStatus::Answered, Some(Codex)));
        let prompt = &r.started()[1].prompt;
        assert!(prompt.contains("rename the client") && prompt.contains("interrupted by the user"), "{prompt}");
        assert!(prompt.ends_with("try again"), "{prompt}");
        assert_eq!(r.engine.sessions.missed_turns(Codex), 0, "told once, then current");
    }

    #[tokio::test]
    async fn a_cancel_between_attempts_keeps_the_real_failure_and_adds_the_interruption() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, QUOTA);
        let cancel = Cancel::new();
        // The failover note is shown after the first attempt failed, before the second starts.
        r.ui.cancel_on_note = Some(cancel.clone());
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        let report = r.engine.run_judged_cancellable("add a retry", judged, &mut r.ui, &cancel).await;
        assert_eq!((report.status, report.attempts), (TurnStatus::Cancelled, 1));
        assert_eq!(r.started().len(), 1, "the second model was never started");

        assert_eq!(r.engine.sessions.turns(), 2, "the failure and the interruption are two records");
        let b = if a == Claude { Codex } else { Claude };
        let text = r.engine.sessions.catch_up(b, "", 10_000).unwrap();
        assert!(text.contains("usage limit reached"), "the real error is kept: {text}");
        assert!(text.contains("interrupted by the user"), "{text}");
        assert!(text.find("usage limit reached") < text.find("interrupted by the user"), "failure first: {text}");
    }

    #[tokio::test]
    async fn a_token_cancelled_before_anything_starts_starts_nothing_and_records_nothing() {
        let mut r = rig(&[Claude]);
        let cancel = Cancel::new();
        cancel.cancel();
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        let report = r.engine.run_judged_cancellable("hello there", judged, &mut r.ui, &cancel).await;
        assert_eq!((report.status, report.attempts), (TurnStatus::Cancelled, 0));
        assert!(r.factory_calls.lock().unwrap().is_empty());
        assert_eq!(r.engine.sessions.turns(), 0, "no model saw the prompt, so there is nothing to tell the next one");
    }

    #[tokio::test]
    async fn cancelling_while_jev_is_being_asked_ends_the_turn_too() {
        let mut r = rig(&[Claude]);
        let cancel = Cancel::new();
        cancel.cancel();
        let report = r.engine.run_cancellable("hello there", &mut r.ui, &cancel).await;
        assert_eq!(report.status, TurnStatus::Cancelled);
        assert!(r.factory_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_turn_takes_the_cli_and_its_helpers_with_it() {
        let dir = std::env::temp_dir().join(format!("jev-cancel-kids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let script: &'static str =
            Box::leak(format!("sleep 60 & echo $! > {}; echo 'TOOL:Read a.rs'; wait", pidfile.display()).into_boxed_str());
        let mut r = rig(&[Claude]);
        r.script(Claude, script);
        let cancel = Cancel::new();
        r.ui.cancel_on_tool = Some(cancel.clone());
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        let report = r.engine.run_judged_cancellable("go", judged, &mut r.ui, &cancel).await;
        assert_eq!(report.status, TurnStatus::Cancelled);
        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        for _ in 0..200 {
            if !backend::process_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!backend::process_alive(pid), "the helper process outlived the cancelled turn");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------ changes and undo

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    struct Repo(PathBuf);

    impl Repo {
        fn new(name: &str, git_init: bool) -> Self {
            let dir = std::env::temp_dir().join(format!("jev-turn-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let dir = std::fs::canonicalize(dir).unwrap();
            std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
            std::fs::write(dir.join("b.txt"), "bee\n").unwrap();
            std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
            if git_init {
                git(&dir, &["init", "-q"]);
                git(&dir, &["add", "-A"]);
                git(&dir, &["commit", "-q", "-m", "init"]);
            }
            Self(dir)
        }
        fn read(&self, f: &str) -> String {
            std::fs::read_to_string(self.0.join(f)).unwrap()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A fake turn that edits a.txt, adds new.txt, deletes b.txt and writes an ignored file.
    const EDITS: &str = "echo three >> a.txt; echo fresh > new.txt; rm b.txt; echo noise > debug.log; printf 'FIN:edited\\n'";

    fn tracked(repo: &Repo, ready: &[ProviderId]) -> Rig {
        let mut r = rig_in(repo.0.clone(), &states_for(ready), OK);
        r.engine = r.engine.with_track_changes(true);
        r
    }

    #[tokio::test]
    async fn a_turn_reports_what_it_changed_and_the_engine_can_diff_and_undo_it() {
        let repo = Repo::new("undo", true);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, EDITS);
        let report = r.turn("edit things", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);

        let sum = report.changes.clone().expect("a git repo");
        let mut paths: Vec<_> = sum.files.iter().map(|f| (f.path.as_str(), f.status)).collect();
        paths.sort_by_key(|p| p.0);
        use crate::changes::ChangeStatus::{Added, Deleted, Modified};
        assert_eq!(paths, [("a.txt", Modified), ("b.txt", Deleted), ("new.txt", Added)], "ignored files are not changes");
        assert_eq!(r.engine.last_changes(), Some(&sum));
        let diff = r.engine.last_diff(None).unwrap();
        assert!(diff.contains("+three") && diff.contains("+++ b/new.txt"), "{diff}");
        assert!(!r.engine.last_diff(Some("new.txt")).unwrap().contains("a.txt"));

        let undone = r.engine.undo_last().unwrap();
        assert_eq!(undone.restored, ["a.txt"]);
        assert_eq!(undone.recreated, ["b.txt"]);
        assert_eq!(undone.removed, ["new.txt"]);
        assert_eq!(repo.read("a.txt"), "one\ntwo\n");
        assert_eq!(repo.read("b.txt"), "bee\n");
        assert!(!repo.0.join("new.txt").exists());
        assert_eq!(repo.read("debug.log"), "noise\n", "ignored files are left alone");

        // One level only, and the conversation knows.
        assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
        assert!(r.engine.last_changes().is_none() && r.engine.last_diff(None).is_none());
        let note = r.engine.sessions.catch_up(Claude, "", 10_000).expect("even the model that ran the turn is told");
        assert!(note.contains("the user undid the changes of the previous turn"), "{note}");
    }

    #[tokio::test]
    async fn undo_refuses_after_a_later_edit_and_keeps_the_summary_for_a_retry() {
        let repo = Repo::new("later", true);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, EDITS);
        r.turn("edit things", want(TaskType::Feature)).await;
        std::fs::write(repo.0.join("a.txt"), "one\ntwo\nthree\nmine\n").unwrap();
        assert_eq!(r.engine.undo_last(), Err(UndoError::ChangedSince(vec!["a.txt".into()])));
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\nmine\n");
        assert!(!repo.0.join("b.txt").exists(), "nothing was undone");
        assert_eq!(r.engine.sessions.turns(), 1, "a refused undo is not recorded");
        assert!(r.engine.last_changes().is_some());
    }

    #[tokio::test]
    async fn a_turn_that_changes_nothing_has_no_summary_and_leaves_nothing_to_undo() {
        let repo = Repo::new("nothing", true);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, EDITS);
        r.turn("edit things", want(TaskType::Feature)).await;
        assert!(r.engine.last_changes().is_some());
        r.script(Claude, OK);
        let report = r.turn("just explain", want(TaskType::Feature)).await;
        assert!(report.changes.is_none());
        assert!(r.engine.last_changes().is_none(), "undo refers to the previous turn, which changed nothing");
        assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
    }

    #[tokio::test]
    async fn a_turn_that_never_started_a_model_keeps_the_earlier_undo() {
        let repo = Repo::new("noroute", true);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, EDITS);
        r.turn("edit things", want(TaskType::Feature)).await;
        r.engine.runtime.cool_down(Claude, None, NOW);
        let report = r.turn("more", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::NoRoute);
        assert!(r.engine.last_changes().is_some());
    }

    #[tokio::test]
    async fn outside_a_git_repository_the_turn_works_and_just_has_no_changes() {
        let repo = Repo::new("nogit", false);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, EDITS);
        let report = r.turn("edit things", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert_eq!(report.text, "edited");
        assert!(report.changes.is_none() && r.engine.last_changes().is_none());
        assert_eq!(r.engine.last_diff(None), None);
        assert_eq!(r.engine.undo_last(), Err(UndoError::NothingToUndo));
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
    }

    #[tokio::test]
    async fn change_tracking_can_be_switched_off() {
        let repo = Repo::new("off", true);
        let mut r = tracked(&repo, &[Claude]);
        r.engine = r.engine.with_track_changes(false);
        r.script(Claude, EDITS);
        assert!(r.turn("edit things", want(TaskType::Feature)).await.changes.is_none());
    }

    #[tokio::test]
    async fn a_cancelled_turn_still_reports_the_files_it_got_to_and_they_can_be_undone() {
        let repo = Repo::new("cancel-edits", true);
        let mut r = tracked(&repo, &[Claude]);
        r.script(Claude, "echo partial > half.txt; echo 'TOOL:Write half.txt'; sleep 30");
        let cancel = Cancel::new();
        r.ui.cancel_on_tool = Some(cancel.clone());
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        let report = r.engine.run_judged_cancellable("start it", judged, &mut r.ui, &cancel).await;
        assert_eq!(report.status, TurnStatus::Cancelled);
        let files = report.changes.expect("a half-done turn is the one you most want to undo").files;
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["half.txt"]);
        r.engine.undo_last().unwrap();
        assert!(!repo.0.join("half.txt").exists());
    }

    // ------------------------------------------------------------------ saved sessions

    struct StateDir(PathBuf);

    impl StateDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("jev-turn-state-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }

    impl Drop for StateDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn persisting(repo: &Repo, state: &StateDir, ready: &[ProviderId]) -> Rig {
        let mut r = rig_in(repo.0.clone(), &states_for(ready), OK);
        r.engine = r.engine.with_persistence(Some(state.0.clone()));
        r
    }

    #[tokio::test]
    async fn a_session_is_saved_after_every_turn_and_resumed_by_a_new_engine() {
        let (repo, state) = (Repo::new("persist", false), StateDir::new("persist"));
        let mut r = persisting(&repo, &state, &[Claude]);
        assert!(r.engine.save_now().is_ok(), "nothing to save yet is not an error");
        assert_eq!(r.engine.session_path(), None, "nothing saved yet, so no file yet");
        r.turn("first request", want(TaskType::Feature)).await;
        r.turn("second request", want(TaskType::Feature)).await;
        let file = r.engine.session_path().unwrap().to_path_buf();
        assert!(file.starts_with(handoff::project_sessions_dir(&state.0, &repo.0)) && file.exists(), "{file:?}");

        // A fresh process: nothing in memory, same folder.
        let mut again = persisting(&repo, &state, &[Claude]);
        assert_eq!(again.engine.sessions.turns(), 0);
        let info = again.engine.resume().unwrap();
        assert_eq!((info.turns, info.providers.clone(), info.last_used_ms), (2, vec![Claude], NOW));
        assert_eq!(info.current.as_ref().map(|c| c.0), Some(Claude));
        assert_eq!(again.engine.routing.current, r.engine.routing.current);
        assert_eq!(again.engine.sessions.session_id(Claude), Some("sess-claude"));
        assert_eq!(again.engine.sessions.recent_context(2, 50), r.engine.sessions.recent_context(2, 50));
        assert_eq!(handoff::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 2);

        // The CLI session is continued, not restarted.
        again.turn("third request", want(TaskType::Feature)).await;
        assert_eq!(again.started()[0].session.as_deref(), Some("sess-claude"));
        assert_eq!(again.engine.sessions.turns(), 3);
    }

    #[tokio::test]
    async fn cancelled_turns_are_saved_too() {
        let (repo, state) = (Repo::new("persist-cancel", false), StateDir::new("persist-cancel"));
        let mut r = persisting(&repo, &state, &[Claude]);
        r.script(Claude, HANGS);
        let cancel = Cancel::new();
        r.ui.cancel_on_tool = Some(cancel.clone());
        let judged = Judged::Providers(Some(want(TaskType::Feature)));
        r.engine.run_judged_cancellable("start it", judged, &mut r.ui, &cancel).await;
        let mut again = persisting(&repo, &state, &[Claude]);
        assert_eq!(again.engine.resume().unwrap().turns, 1);
        assert!(again.engine.sessions.catch_up(Codex, "", 1000).unwrap().contains("interrupted by the user"));
    }

    #[tokio::test]
    async fn persistence_is_off_unless_asked_for() {
        let repo = Repo::new("persist-off", false);
        let mut r = rig_in(repo.0.clone(), &states_for(&[Claude]), OK);
        r.turn("hello", want(TaskType::Feature)).await;
        assert_eq!(r.engine.session_path(), None);
        assert!(r.engine.save_now().is_ok());
        assert_eq!(r.engine.resume().unwrap_err(), ResumeError::Disabled);
    }

    #[tokio::test]
    async fn new_session_starts_another_file_and_leaves_the_old_one_resumable() {
        let (repo, state) = (Repo::new("persist-new", false), StateDir::new("persist-new"));
        let mut r = persisting(&repo, &state, &[Claude]);
        r.turn("hello", want(TaskType::Feature)).await;
        let old = r.engine.session_path().unwrap().to_path_buf();
        r.engine.new_session();
        assert!(old.exists(), "/new no longer deletes the saved session");
        assert_eq!(r.engine.session_path(), None);
        r.turn("something else", want(TaskType::Feature)).await;
        let new = r.engine.session_path().unwrap().to_path_buf();
        assert_ne!(old, new);
        assert_eq!(r.engine.sessions.turns(), 1, "the new session starts empty");
        // The old one is what /resume brings back.
        assert_eq!(r.engine.resume().unwrap().turns, 1);
        assert_eq!(r.engine.session_path(), Some(old.as_path()));
        assert!(r.engine.sessions.recent_context(1, 50).contains("hello"));
    }

    /// A session of one prompt, saved by an engine of its own (a process of its own).
    async fn session_of(repo: &Repo, state: &StateDir, prompts: &[&str], at: u64) -> Rig {
        let mut r = persisting(repo, state, &[Claude]);
        r.clock.store(at, Ordering::SeqCst);
        for p in prompts {
            r.turn(p, want(TaskType::Feature)).await;
        }
        r
    }

    #[tokio::test]
    async fn a_new_sessions_first_turn_does_not_overwrite_the_previous_one() {
        let (repo, state) = (Repo::new("persist-keep", false), StateDir::new("persist-keep"));
        let first = session_of(&repo, &state, &["one", "two", "three"], NOW).await;
        assert_eq!(handoff::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 3);

        // The startup offer was `previous session: 3 turns`; the user ignores it and types.
        let second = session_of(&repo, &state, &["a new topic"], NOW + 1000).await;
        assert_ne!(first.engine.session_path(), second.engine.session_path());

        // The 3-turn conversation is intact: it is what is left after the newer one.
        let mut third = persisting(&repo, &state, &[Claude]);
        third.clock.store(NOW + 2000, Ordering::SeqCst);
        assert_eq!(third.engine.resume().unwrap().turns, 1, "the most recent session comes first");
        assert_eq!(third.engine.resume().unwrap().turns, 3, "and the one before it was not destroyed");
        assert!(third.engine.sessions.recent_context(3, 50).contains("three"));
    }

    #[tokio::test]
    async fn resuming_continues_the_resumed_file_and_a_further_resume_steps_back() {
        let (repo, state) = (Repo::new("persist-step", false), StateDir::new("persist-step"));
        session_of(&repo, &state, &["old one", "old two"], NOW).await;
        session_of(&repo, &state, &["recent"], NOW + 1000).await;
        let mut r = persisting(&repo, &state, &[Claude]);
        r.clock.store(NOW + 2000, Ordering::SeqCst);
        r.engine.resume().unwrap();
        let recent = r.engine.session_path().unwrap().to_path_buf();
        r.turn("and more", want(TaskType::Feature)).await;
        assert_eq!(r.engine.session_path(), Some(recent.as_path()), "a resumed session is continued in place, not copied");
        assert_eq!(std::fs::read_dir(recent.parent().unwrap()).unwrap().count(), 2, "no third file");
        assert_eq!(r.engine.resume().unwrap().turns, 2, "the older session is next; the running one is skipped");
        assert_ne!(r.engine.session_path(), Some(recent.as_path()));
        // With only two sessions, the running one is always skipped, so it alternates.
        r.engine.resume().unwrap();
        assert_eq!(r.engine.session_path(), Some(recent.as_path()));
    }

    #[tokio::test]
    async fn the_resume_list_is_newest_first_named_by_the_first_message_and_any_can_be_picked() {
        let (repo, state) = (Repo::new("persist-list", false), StateDir::new("persist-list"));
        session_of(&repo, &state, &["fix the parser", "and its tests"], NOW).await;
        session_of(&repo, &state, &["rename the crate"], NOW + 1000).await;
        let mut r = persisting(&repo, &state, &[Claude]);
        r.clock.store(NOW + 2000, Ordering::SeqCst);
        let list = r.engine.resumable().unwrap();
        let shown: Vec<_> = list.iter().map(|e| (e.title.as_str(), e.info.turns)).collect();
        assert_eq!(shown, vec![("rename the crate", 1), ("fix the parser", 2)]);
        // The older one, not the newest, is taken; and it is continued in place.
        let info = r.engine.resume_from(&list[1].path).unwrap();
        assert_eq!(info.turns, 2);
        assert_eq!(r.engine.session_path(), Some(list[1].path.as_path()));
        // The running session is not offered again.
        let after: Vec<_> = r.engine.resumable().unwrap().iter().map(|e| e.title.clone()).collect();
        assert_eq!(after, vec!["rename the crate".to_string()]);
    }

    #[tokio::test]
    async fn the_resume_list_leaves_out_other_folders_and_missing_files_fail_cleanly() {
        let (repo, other, state) = (Repo::new("persist-list-a", false), Repo::new("persist-list-b", false), StateDir::new("persist-list-ab"));
        session_of(&other, &state, &["someone else's work"], NOW).await;
        let mut r = persisting(&repo, &state, &[Claude]);
        assert!(r.engine.resumable().unwrap().is_empty());
        let gone = state.0.join("sessions").join("nothing.json");
        assert_eq!(r.engine.resume_from(&gone).unwrap_err(), ResumeError::NotFound);
    }

    #[tokio::test]
    async fn only_the_ten_newest_sessions_of_a_project_are_kept() {
        let (repo, state) = (Repo::new("persist-cap", false), StateDir::new("persist-cap"));
        let mut files = Vec::new();
        for i in 0..12u64 {
            let r = session_of(&repo, &state, &["hello"], NOW + i * 1000).await;
            files.push(r.engine.session_path().unwrap().to_path_buf());
        }
        let dir = handoff::project_sessions_dir(&state.0, &repo.0);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), handoff::SAVED_KEEP);
        assert!(!files[0].exists() && !files[1].exists(), "the oldest went");
        assert!(files[2..].iter().all(|f| f.exists()));
    }

    #[tokio::test]
    async fn a_session_saved_by_an_older_version_in_one_file_is_still_resumed() {
        let (repo, state) = (Repo::new("persist-legacy", false), StateDir::new("persist-legacy"));
        let donor = session_of(&repo, &state, &["from before"], NOW).await;
        // Move that session to where the old version kept it.
        let legacy = handoff::legacy_session_file(&state.0, &repo.0);
        std::fs::rename(donor.engine.session_path().unwrap(), &legacy).unwrap();
        assert_eq!(handoff::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 1);
        let mut r = persisting(&repo, &state, &[Claude]);
        assert_eq!(r.engine.resume().unwrap().turns, 1);
        r.turn("carry on", want(TaskType::Feature)).await;
        assert_eq!(r.engine.sessions.turns(), 2);
        assert_eq!(std::fs::read_dir(handoff::project_sessions_dir(&state.0, &repo.0)).map_or(0, Iterator::count), 0);
    }

    #[tokio::test]
    async fn old_sessions_are_not_resumed_and_projects_do_not_see_each_other() {
        let (repo, other, state) = (Repo::new("persist-age", false), Repo::new("persist-other", false), StateDir::new("persist-age"));
        let mut r = persisting(&repo, &state, &[Claude]);
        r.turn("hello", want(TaskType::Feature)).await;

        let mut elsewhere = persisting(&other, &state, &[Claude]);
        assert_eq!(elsewhere.engine.resume().unwrap_err(), ResumeError::NotFound);

        let mut later = persisting(&repo, &state, &[Claude]);
        later.clock.store(NOW + 15 * 24 * 60 * 60 * 1000, Ordering::SeqCst);
        assert_eq!(later.engine.resume().unwrap_err(), ResumeError::NotFound);
        later.clock.store(NOW + 13 * 24 * 60 * 60 * 1000, Ordering::SeqCst);
        assert!(later.engine.resume().is_ok());
    }

    #[tokio::test]
    async fn a_garbage_saved_file_is_reported_and_a_new_session_saves_beside_it() {
        let (repo, state) = (Repo::new("persist-bad", false), StateDir::new("persist-bad"));
        let mut r = persisting(&repo, &state, &[Claude]);
        let dir = handoff::project_sessions_dir(&state.0, &repo.0);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1-1.json"), "{ this is not json").unwrap();
        assert!(matches!(r.engine.resume(), Err(ResumeError::Corrupt(_))));
        assert_eq!(r.turn("hello", want(TaskType::Feature)).await.status, TurnStatus::Answered);
        let mut again = persisting(&repo, &state, &[Claude]);
        assert_eq!(again.engine.resume().unwrap().turns, 1, "the good session is found past the damaged file");
    }

    #[tokio::test]
    async fn a_save_that_cannot_be_written_never_fails_the_turn_but_save_now_says_why() {
        let repo = Repo::new("persist-ro", false);
        // A regular file where the state directory should be.
        let blocker = std::env::temp_dir().join(format!("jev-turn-blocker-{}", std::process::id()));
        std::fs::write(&blocker, "x").unwrap();
        let mut r = rig_in(repo.0.clone(), &states_for(&[Claude]), OK);
        r.engine = r.engine.with_persistence(Some(blocker.join("state")));
        let report = r.turn("hello", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert!(r.engine.save_now().is_err());
        std::fs::remove_file(&blocker).ok();
    }
    /// A UI that overrides every structured callback and records what each one got.
    #[derive(Default)]
    struct Structured {
        log: Vec<String>,
    }

    impl Ui for Structured {
        fn route(&mut self, _: &str) {
            self.log.push("route-string".into());
        }
        fn event(&mut self, _: &Event) {}
        fn note(&mut self, line: &str) {
            self.log.push(format!("note:{line}"));
        }
        fn route_facts(&mut self, f: &RouteFacts) {
            self.log.push(format!("route:{}:{}:{}", f.provider, f.model, f.task));
        }
        fn failover(&mut self, f: &Failover) {
            self.log.push(format!("failover:{}:{:?}:retry={}", f.provider, f.kind, f.will_retry));
        }
        fn no_route(&mut self, info: &NoRouteInfo, _: &str) {
            self.log.push(match info {
                NoRouteInfo::Pin { .. } => "no-route:pin".into(),
                NoRouteInfo::Nothing { task } => format!("no-route:{task}"),
                NoRouteInfo::Refused { .. } => "no-route:refused".into(),
            });
        }
        fn cancelled(&mut self, _: &str) {
            self.log.push("cancelled".into());
        }
    }

    #[tokio::test]
    async fn structured_callbacks_carry_the_facts_and_replace_the_string_ones() {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, QUOTA);
        let mut ui = Structured::default();
        let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
        assert_eq!(report.status, TurnStatus::Answered);
        let b = report.provider.unwrap();
        assert_eq!(ui.log.len(), 3, "{:?}", ui.log);
        assert!(ui.log[0].starts_with(&format!("route:{a}:")) && ui.log[0].ends_with(":feature"), "{:?}", ui.log);
        assert_eq!(ui.log[1], format!("failover:{a}:Quota {{ resets_at: Some(\"1794853451\") }}:retry=true"));
        assert!(ui.log[2].starts_with(&format!("route:{b}:")), "{:?}", ui.log);
        assert!(!ui.log.iter().any(|l| l == "route-string"), "the string callback is not used when the structured one is overridden");
    }

    #[tokio::test]
    async fn a_final_failure_and_a_missing_route_are_reported_structurally() {
        let mut r = rig(&[Claude]);
        r.script(Claude, OTHER);
        let mut ui = Structured::default();
        let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
        assert_eq!(report.status, TurnStatus::Failed);
        assert_eq!(ui.log.last().unwrap(), "failover:claude:Other:retry=false");

        let mut none = rig(&[]);
        let mut ui = Structured::default();
        let report = none.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
        assert_eq!(report.status, TurnStatus::NoRoute);
        assert_eq!(ui.log, vec!["no-route:feature".to_string()]);
    }

    #[test]
    fn idle_timeout_reads_positive_seconds_and_defaults_otherwise() {
        assert_eq!(idle_timeout(None), Duration::from_secs(900));
        assert_eq!(idle_timeout(Some(" 30 ")), Duration::from_secs(30));
        for bad in ["", "abc", "0", "-5", "1.5"] {
            assert_eq!(idle_timeout(Some(bad)), Duration::from_secs(900), "{bad:?}");
        }
    }

    // ------------------------------------------------------------ tool approvals

    /// An interactive "CLI": asks to run `cargo test --quiet` through Bash and reports the answer.
    struct AskFake(&'static str);

    const ASK_SCRIPT: &str = r#"read p; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#DENY:}";; esac"#;

    impl Backend for AskFake {
        fn provider(&self) -> ProviderId {
            Claude
        }
        fn command(&self, req: &TurnRequest) -> Command {
            let mut c = Command::new("sh");
            c.arg("-c").arg(self.0).current_dir(&req.dir);
            c
        }
        fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
            Some(format!("{}\n", req.prompt))
        }
        fn interactive(&self) -> bool {
            true
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            match line.trim_end().split_once(':') {
                Some(("ASK", rest)) => {
                    let (tool, summary) = rest.split_once(':').unwrap_or((rest, ""));
                    vec![Event::PermissionRequest(PermissionRequest {
                        id: "q1".into(),
                        tool: tool.into(),
                        summary: summary.into(),
                        input: json!({ "command": summary }),
                        blocked_path: None,
                    })]
                }
                Some(("WITHDRAW", id)) => vec![Event::PermissionWithdrawn { id: id.into() }],
                Some(("FIN", t)) => vec![Event::Finished(backend::Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn permission_response(&self, _: &PermissionRequest, d: &Decision) -> Option<String> {
            Some(match d {
                Decision::Allow | Decision::AllowAlways => "ALLOW".to_string(),
                Decision::Deny { reason } => format!("DENY:{reason}"),
            })
        }
        fn classify_error(&self, m: &str) -> ErrorKind {
            classify_common(m).unwrap_or(ErrorKind::Other)
        }
    }

    /// A UI that answers questions from a list, and records what it was asked and told.
    #[derive(Default)]
    struct Asked {
        answers: std::collections::VecDeque<Option<Decision>>,
        questions: Vec<(String, String, Option<String>)>,
        settled: Vec<Settled>,
        texts: Vec<String>,
        notes: Vec<String>,
        hints: Vec<String>,
    }

    impl Ui for Asked {
        fn route(&mut self, _: &str) {}
        fn event(&mut self, e: &Event) {
            match e {
                Event::Text(t) => self.texts.push(t.clone()),
                Event::Hint(h) => self.hints.push(h.clone()),
                _ => {}
            }
        }
        fn note(&mut self, line: &str) {
            self.notes.push(line.into());
        }
        fn ask_permission(&mut self, req: &PermissionRequest, offer: &Offer) -> Answer {
            let rule = offer.as_ref().ok().and_then(|a| a.rules.first().cloned());
            self.questions.push((req.tool.clone(), req.summary.clone(), rule));
            let answer = self.answers.pop_front().expect("an answer was planned");
            Box::pin(std::future::ready(answer))
        }
        fn permission_settled(&mut self, req: &PermissionRequest, settled: &Settled) {
            self.settled.push(settled.clone());
            if let Settled::NotSaved(e) = settled {
                self.note(&format!("[permissions] {e}; allowed this once, nothing was saved ({})", req.tool));
            }
        }
    }

    /// A UI that keeps the default `ask_permission`: nobody to ask.
    #[derive(Default)]
    struct Mute {
        texts: Vec<String>,
        hints: Vec<String>,
    }

    impl Ui for Mute {
        fn route(&mut self, _: &str) {}
        fn event(&mut self, e: &Event) {
            match e {
                Event::Text(t) => self.texts.push(t.clone()),
                Event::Hint(t) => self.hints.push(t.clone()),
                _ => {}
            }
        }
        fn note(&mut self, _: &str) {}
    }

    fn ask_engine(name: &str) -> (Engine, PathBuf) {
        ask_engine_running(name, Box::new(|_| Box::new(AskFake(ASK_SCRIPT))))
    }

    fn ask_engine_running(name: &str, backend: BackendFactory) -> (Engine, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jev-turn-ask-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let router = Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0);
        let engine = Engine::new(Runtime::for_tests(vec![availability(Claude, State::Ready)]), router, dir.clone())
            .with_track_changes(false)
            .with_persistence(None)
            .with_backend_factory(backend)
            .with_idle_timeout(Duration::from_secs(5))
            .with_turn_log(None);
        (engine, dir)
    }

    async fn ask_turn(engine: &mut Engine, ui: &mut dyn Ui) -> TurnReport {
        engine.run_judged("run the tests", Judged::Providers(Some(want(TaskType::Feature))), ui).await
    }

    #[tokio::test]
    async fn an_allowed_tool_call_runs_and_the_person_was_asked_once_with_the_rule_on_offer() {
        let (mut engine, dir) = ask_engine("allow");
        let mut ui = Asked { answers: [Some(Decision::Allow)].into(), ..Asked::default() };
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
        assert_eq!(ui.questions, vec![("Bash".to_string(), "cargo test --quiet".to_string(), Some("Bash(cargo test:*)".to_string()))]);
        assert!(!permissions::local_settings_path(&dir).exists(), "a plain yes saves nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auto_mode_runs_a_safe_call_without_asking_and_saves_nothing() {
        let (engine, dir) = ask_engine("auto-safe");
        let mut engine = engine.with_mode(Mode::Auto);
        let mut ui = Asked::default();
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
        assert!(ui.questions.is_empty(), "nobody was asked: {:?}", ui.questions);
        assert!(ui.hints.iter().any(|h| h.contains("auto-allowed") && h.contains("cargo test --quiet")), "{:?}", ui.hints);
        assert!(!permissions::local_settings_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auto_mode_allows_a_call_that_cannot_be_remembered() {
        let script = r#"read p; echo "ASK:Bash:git push"; read ans; case "$ans" in ALLOW) echo "FIN:ran";; *) echo "FIN:refused";; esac"#;
        let (engine, dir) = ask_engine_running("auto-push", Box::new(move |_| Box::new(AskFake(script))));
        let mut engine = engine.with_mode(Mode::Auto);
        let mut ui = Asked::default();
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
        assert!(ui.questions.is_empty());
        assert!(ui.hints.iter().any(|h| h.contains("auto-allowed") && h.contains("git push")));
        assert!(!permissions::local_settings_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_other_modes_still_ask_about_the_same_call() {
        for mode in [Mode::Ask, Mode::Edit, Mode::Plan] {
            let (engine, dir) = ask_engine(&format!("ask-{mode}"));
            let mut engine = engine.with_mode(mode);
            let mut ui = Asked { answers: [Some(Decision::Allow)].into(), ..Asked::default() };
            ask_turn(&mut engine, &mut ui).await;
            assert_eq!(ui.questions.len(), 1, "{mode}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[tokio::test]
    async fn always_runs_the_call_and_saves_the_rule_in_the_project() {
        let (mut engine, dir) = ask_engine("always");
        let mut ui = Asked { answers: [Some(Decision::AllowAlways)].into(), ..Asked::default() };
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!(report.text, "ran");
        let saved = permissions::load(&permissions::local_settings_path(&dir)).unwrap();
        assert_eq!(saved.list(permissions::List::Allow), ["Bash(cargo test:*)"]);
        assert!(ui.notes.is_empty(), "{:?}", ui.notes);
        let always = permissions::Always { rules: vec!["Bash(cargo test:*)".into()], folder: None };
        assert_eq!(ui.settled, [Settled::Remembered(always)], "the UI hears what was saved, after the save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn always_that_cannot_be_saved_still_allows_once_and_says_so() {
        let (mut engine, dir) = ask_engine("always-blocked");
        // `.claude` is a file, so the settings file cannot be created.
        std::fs::write(dir.join(".claude"), "x").unwrap();
        let mut ui = Asked { answers: [Some(Decision::AllowAlways)].into(), ..Asked::default() };
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!(report.text, "ran");
        assert!(ui.notes.iter().any(|n| n.contains("[permissions]") && n.contains("nothing was saved")), "{:?}", ui.notes);
        assert!(matches!(ui.settled.as_slice(), [Settled::NotSaved(_)]), "never told 'always' when nothing was saved: {:?}", ui.settled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_denied_tool_call_tells_the_model_why_and_the_turn_goes_on() {
        let (mut engine, dir) = ask_engine("deny");
        let denial = Decision::Deny { reason: "the user denied this tool call".into() };
        let mut ui = Asked { answers: [Some(denial)].into(), ..Asked::default() };
        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "refused the user denied this tool call"));
        assert!(!permissions::local_settings_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ctrl_c_at_the_question_cancels_the_turn_and_kills_the_cli() {
        let (mut engine, dir) = ask_engine("interrupt");
        let mut ui = Asked { answers: [None].into(), ..Asked::default() };
        let cancel = Cancel::new();
        let report = engine
            .run_judged_cancellable("run the tests", Judged::Providers(Some(want(TaskType::Feature))), &mut ui, &cancel)
            .await;
        assert_eq!(report.status, TurnStatus::Cancelled);
        assert!(cancel.is_cancelled());
        assert!(report.text.is_empty());
        assert_eq!(ui.questions.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_ui_that_cannot_ask_denies_at_once_with_a_hint_and_never_hangs() {
        let (mut engine, dir) = ask_engine("mute");
        let mut ui = Mute::default();
        let started = Instant::now();
        let report = ask_turn(&mut engine, &mut ui).await;
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(report.status, TurnStatus::Answered);
        assert!(report.text.starts_with("refused no one to ask"), "{}", report.text);
        assert!(ui.hints.iter().any(|t| t.contains("denied Bash") && t.contains("CODE_ALLOWED_TOOLS")), "{:?}", ui.hints);
        assert!(ui.texts.iter().all(|t| !t.contains("denied Bash")), "a hint is never answer text: {:?}", ui.texts);
        assert!(!permissions::local_settings_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backends_without_a_host_protocol_are_never_asked() {
        let mut r = rig(&[Claude]);
        let mut ui = Asked::default();
        let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
        assert_eq!(report.status, TurnStatus::Answered);
        assert!(ui.questions.is_empty());
    }

    /// Records that the wait for the person's answer was dropped.
    struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A UI whose first question waits for a key that never comes (the person is thinking),
    /// and which answers yes to any later one. It records the wait being dropped and the
    /// withdrawal it was told about.
    #[derive(Default)]
    struct Waiting {
        asked: usize,
        dropped: Arc<std::sync::atomic::AtomicBool>,
        withdrawn: Vec<String>,
    }

    impl Ui for Waiting {
        fn route(&mut self, _: &str) {}
        fn event(&mut self, _: &Event) {}
        fn note(&mut self, _: &str) {}
        fn ask_permission(&mut self, _: &PermissionRequest, _: &Offer) -> Answer {
            self.asked += 1;
            if self.asked > 1 {
                return Box::pin(std::future::ready(Some(Decision::Allow)));
            }
            let flag = DropFlag(self.dropped.clone());
            Box::pin(async move {
                let _waiting = flag;
                std::future::pending::<Option<Decision>>().await
            })
        }
        fn permission_withdrawn(&mut self, req: &PermissionRequest) {
            self.withdrawn.push(req.id.clone());
        }
    }

    /// The CLI asks, the person has not answered yet, and the CLI withdraws the question.
    const ASK_THEN_WITHDRAW: &str = r#"read p; echo "ASK:Bash:cargo test --quiet"; sleep 0.3; echo "WITHDRAW:q1"; sleep 0.1; echo "FIN:carried on""#;

    #[tokio::test]
    async fn a_question_the_cli_withdraws_while_the_person_is_still_deciding_is_dropped_and_the_next_turn_works() {
        let turns = Arc::new(AtomicU64::new(0));
        let counted = turns.clone();
        let (mut engine, dir) = ask_engine_running(
            "withdrawn",
            Box::new(move |_| Box::new(AskFake(if counted.fetch_add(1, Ordering::SeqCst) == 0 { ASK_THEN_WITHDRAW } else { ASK_SCRIPT }))),
        );
        let mut ui = Waiting::default();
        let started = Instant::now();
        let report = ask_turn(&mut engine, &mut ui).await;
        assert!(started.elapsed() < Duration::from_secs(4), "the turn must not wait for a key");
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "carried on"));
        assert_eq!(ui.asked, 1);
        assert!(ui.dropped.load(Ordering::SeqCst), "the wait for a key was abandoned");
        assert_eq!(ui.withdrawn, vec!["q1".to_string()], "the UI was told which question went away");
        assert!(!permissions::local_settings_path(&dir).exists());

        let report = ask_turn(&mut engine, &mut ui).await;
        assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
        assert_eq!(ui.asked, 2);
        assert_eq!(ui.withdrawn.len(), 1, "an answered question is never reported as withdrawn");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
