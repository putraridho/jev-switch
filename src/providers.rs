//! Turning "which provider" into a runnable backend, and remembering what the
//! router has learned about providers while it runs (who is logged in, who hit a
//! limit, which models were refused).
//!
//! Choices are pure functions ([`choose_gemini_flavor`], [`deepseek_env`],
//! [`usable_from`], [`allowances_for`], [`format_left`]); the few IO calls (PATH,
//! settings file, environment) sit in thin wrappers beside them so the rules can be
//! tested without a CLI, a network or the user's home directory.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::availability::{self, Availability, State};
use crate::backend::claude::ClaudeBackend;
use crate::backend::codex::CodexBackend;
use crate::backend::gemini::{Flavor, GeminiBackend};
pub use crate::backend::select::{choose_gemini_flavor, deepseek_env, gemini_env};
use crate::backend::grok::GrokBackend;
use crate::backend::{Backend, ErrorKind, PlanUsage, ProviderId, TurnError};
use crate::catalog::{self, Allowances, Overrides, Usable};
use crate::cooldown::{self, Cooldowns, DiskWatch};
use crate::envfile;
use crate::perf::ProbeSource;
use crate::probecache;

/// Every provider, in the order `/providers` lists them.
pub const ALL_PROVIDERS: [ProviderId; 5] =
    [ProviderId::Claude, ProviderId::DeepSeek, ProviderId::Codex, ProviderId::Gemini, ProviderId::Grok];

const MINUTE_MS: u64 = 60_000;

/// Where keys live when `$JEV_ROUTER_ENV` does not say otherwise; shown in fixes.
pub const DEFAULT_ENV_PATH: &str = "~/.jev-router.env";

// ------------------------------------------------------------------ backends

/// A fresh backend for one turn. Codex and Gemini keep per-turn parse state inside
/// the instance, so sharing one across turns would mix their output: build a new one
/// every time (construction is cheap).
pub fn backend_for(provider: ProviderId) -> Box<dyn Backend> {
    build_backend(provider, false)
}

/// [`backend_for`] for the `jev-code` session: the Claude Code CLIs (Claude, DeepSeek)
/// ask the host about tool approvals and know where they run. The other CLIs are the same
/// as in [`backend_for`]: they have no host approval protocol, Codex's sandbox decides.
pub fn session_backend_for(provider: ProviderId) -> Box<dyn Backend> {
    build_backend(provider, true)
}

/// The backends of a `jev-code` run: [`session_backend_for`] when someone can answer an
/// approval question (`can_ask`: a session on a terminal), else [`backend_for`]. A run nobody
/// can answer (`-p`, piped input) must not use the host prompt protocol: every question would
/// be denied, and the model would be told "jev-code asks the user" when nobody will. Plain
/// `claude -p` decides with the allowed tools and the permission mode instead.
pub fn run_backends(can_ask: bool) -> fn(ProviderId) -> Box<dyn Backend> {
    if can_ask {
        session_backend_for
    } else {
        backend_for
    }
}

fn build_backend(provider: ProviderId, session: bool) -> Box<dyn Backend> {
    let claude_code = |backend: ClaudeBackend| -> Box<dyn Backend> {
        Box::new(if session { backend.in_session() } else { backend })
    };
    match provider {
        ProviderId::Claude => claude_code(ClaudeBackend::claude()),
        ProviderId::Codex => Box::new(CodexBackend::codex()),
        ProviderId::Grok => Box::new(GrokBackend::grok().with_env(crate::backend::grok::auth_env(&envfile::lookup))),
        ProviderId::Gemini => Box::new(match detect_gemini_flavor() {
            Flavor::GeminiCli => GeminiBackend::gemini_cli().with_env(gemini_env(&envfile::lookup)),
            Flavor::Antigravity => GeminiBackend::antigravity().with_env(gemini_env(&envfile::lookup)),
        }),
        // No key means the caller never routes here; an empty key just fails the turn.
        ProviderId::DeepSeek => {
            let key = envfile::lookup("DEEPSEEK_API_KEY").unwrap_or_default();
            claude_code(ClaudeBackend::with_profile(ProviderId::DeepSeek, deepseek_env(&key)))
        }
    }
}

/// The CLI behind `gemini` right now, for display: `gemini` or `agy`.
pub fn gemini_via() -> &'static str {
    match detect_gemini_flavor() {
        Flavor::GeminiCli => "gemini",
        Flavor::Antigravity => "agy",
    }
}

fn detect_gemini_flavor() -> Flavor {
    let settings = std::env::var_os("HOME")
        .and_then(|h| std::fs::read_to_string(PathBuf::from(h).join(".gemini/settings.json")).ok());
    // The same rule the availability probe uses, so "ready" and "which CLI" agree.
    let configured = crate::backend::gemini::is_configured(settings.as_deref(), |n| envfile::lookup(n).is_some());
    choose_gemini_flavor(crate::availability::installed_for_real(&GeminiBackend::gemini_cli().bin), configured)
}

/// How to install the CLI behind `provider`: the one source of install hints.
pub fn install_hint(provider: ProviderId) -> &'static str {
    match provider {
        // DeepSeek is the claude CLI pointed at another API.
        ProviderId::Claude | ProviderId::DeepSeek => "Install Claude Code: https://code.claude.com/docs/en/quickstart",
        ProviderId::Codex => "npm i -g @openai/codex (or see the vendor's install page)",
        ProviderId::Gemini => "npm i -g @google/gemini-cli",
        ProviderId::Grok => {
            "Install the Grok CLI: see the vendor's install page (if `grok` is a lazy mise wrapper, running `grok` once installs it)"
        }
    }
}

/// The command that logs in to `provider`: the one source of login commands, used by
/// `doctor`, the first-run hint and every error message. `None` for DeepSeek, which has
/// no login: it takes an API key in the env file. Only `claude auth login` and `codex
/// login` were tried against the real CLIs ([`login_verified`]).
pub fn login_command(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some("claude auth login"),
        ProviderId::Codex => Some("codex login"),
        // Running the CLI starts its own sign-in flow.
        ProviderId::Gemini => Some("gemini"),
        ProviderId::Grok => Some("grok"),
        ProviderId::DeepSeek => None,
    }
}

/// Whether [`login_command`] for `provider` was run against the real CLI.
pub fn login_verified(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex)
}

/// [`login_command`] as one sentence for a message: `run `codex login``.
pub fn login_instruction(provider: ProviderId) -> String {
    match (provider, login_command(provider)) {
        (ProviderId::Gemini, Some(cmd)) => format!("run `{cmd}` and choose a login"),
        (ProviderId::Grok, Some(cmd)) => format!("run `{cmd}` and sign in"),
        (_, Some(cmd)) => format!("run `{cmd}`"),
        (_, None) => format!("add `DEEPSEEK_API_KEY` to {DEFAULT_ENV_PATH}"),
    }
}

// ------------------------------------------------------------- pure decisions

/// Credits are opt-in via `JEV_ALLOW_CREDITS` ("1" or "true"). Per-token models are
/// allowed exactly when DeepSeek is Ready, because configuring its key is the
/// user's agreement to pay per token.
pub fn allowances_for(credits_env: Option<&str>, avail: &[Availability]) -> Allowances {
    Allowances {
        // Extra-credit models (Claude Fable) are free to use when the plan includes them:
        // Claude Max and Ultra do, Pro bills them as pay-as-you-go credits (opt-in).
        credits: credits_env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
            || claude_plan_includes_fable(avail),
        per_token: deepseek_ready(avail),
    }
}

/// Whether the logged-in Claude plan includes Fable (Max and Ultra do; Pro does not).
fn claude_plan_includes_fable(avail: &[Availability]) -> bool {
    avail.iter().any(|a| {
        a.provider == ProviderId::Claude
            && a.state == State::Ready
            && a.plan.as_deref().is_some_and(|p| {
                let p = p.to_ascii_lowercase();
                p.contains("max") || p.contains("ultra")
            })
    })
}

fn deepseek_ready(avail: &[Availability]) -> bool {
    avail.iter().any(|a| a.provider == ProviderId::DeepSeek && a.state == State::Ready)
}

/// The models a turn may go to: those of Ready providers (narrowed by the login's
/// own model list), minus providers cooling down and models a provider refused.
pub fn usable_from(
    avail: &[Availability],
    cooldowns: &Cooldowns,
    unusable: &HashSet<(ProviderId, String)>,
    now_ms: u64,
) -> Usable {
    let catalog_ids = |p: ProviderId| {
        catalog::models().iter().filter(|m| m.provider == p).map(|m| m.id.to_string()).collect::<Vec<_>>()
    };
    Usable::new(
        availability::usable_models(avail, &catalog_ids)
            .into_iter()
            .filter(|(p, _)| cooldowns.active(*p, now_ms).is_none())
            .filter(|pair| !unusable.contains(pair)),
    )
}

/// "12m", "3h 5m", "2d 4h": short enough for one `/providers` line. Under a minute
/// reads "<1m" so a nearly finished cooldown is not shown as zero.
pub fn format_left(ms: u64) -> String {
    let minutes = ms / MINUTE_MS;
    match minutes {
        0 => "<1m".to_string(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

// -------------------------------------------------------------------- Runtime

/// What the router knows about providers right now. Availability comes from probes;
/// cooldowns persist across runs; unusable models and plan utilization live only for
/// this run.
#[derive(Debug)]
pub struct Runtime {
    pub avail: Vec<Availability>,
    pub cooldowns: Cooldowns,
    pub allow: Allowances,
    pub overrides: Overrides,
    /// Where cooldowns are saved; `None` when there is no state directory.
    pub cooldown_path: Option<PathBuf>,
    /// The last copy of the cooldown file read, so limits other processes saved
    /// (`jev-code`, `delegate_mcp`) are seen without a restart. Behind a mutex because
    /// [`Runtime::usable`] only has `&self`.
    disk: Mutex<DiskWatch>,
    /// Models a provider refused at run time (e.g. not entitled), so they are not retried.
    pub unusable_models: HashSet<(ProviderId, String)>,
    /// The latest plan utilization each provider reported.
    pub plan: HashMap<ProviderId, PlanUsage>,
    /// Where `plan` is saved after every new reading; `None` in tests and without a state dir.
    plan_path: Option<PathBuf>,
    /// DeepSeek's remaining account balance, filled in by a background fetch (it is pay per
    /// token, so a balance is what "credit left" means there, not a plan percentage).
    balance: std::sync::Arc<Mutex<Option<crate::balance::Balance>>>,
    /// Providers that failed in a way another provider can cover and that is not a usage
    /// limit, with why. Session-only and kept out of the persisted cooldowns on purpose:
    /// a login problem or an unusable directory is not a limit, and an hour-long cooldown
    /// would only outlast a re-login. See [`BlockReason`] for what clears each.
    pub blocked: HashMap<ProviderId, BlockReason>,
    /// The background retry of probes that timed out at startup, until
    /// [`Runtime::absorb_late`] has folded its answers into `avail`.
    late: Option<LateProbes>,
}

/// Answers from the background retry, and what to tell the user about the ones applied.
struct LateProbes {
    answers: tokio::sync::mpsc::UnboundedReceiver<Availability>,
    /// What an answer may change: only a provider still Unknown (the slow-probe retry), or
    /// any provider whose answer differs from the cached one the start used.
    merge: LateMerge,
    /// Called from `absorb_late`, only for a provider that really became Ready.
    on_ready: Box<dyn Fn(&Availability) + Send + Sync>,
}

/// How a background answer is folded into the availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LateMerge {
    /// [`availability::merge_late`].
    FillUnknown,
    /// [`availability::merge_refresh`].
    Refresh,
}

/// The cache a start may read, and keep up to date.
#[derive(Debug, Clone)]
pub struct ProbeCache {
    pub path: PathBuf,
    pub fingerprint: u64,
}

impl fmt::Debug for LateProbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LateProbes")
    }
}

/// Why a provider is skipped for the rest of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The login failed. `/providers refresh` clears it once the probe sees a login.
    Auth,
    /// The CLI refuses to run in this directory (Codex outside a git repo, Gemini in an
    /// untrusted workspace). Probing cannot change that, so only `/new` or another
    /// project directory (a new session) clears it.
    UnusableHere,
}

/// What the user can do about [`BlockReason::UnusableHere`] for `provider`.
fn unusable_here_hint(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "needs a git repository here (git init, or work in a repo)",
        ProviderId::Gemini => "needs a trusted workspace here (run gemini in this directory once to trust it)",
        _ => "refuses to run in this directory",
    }
}

impl Runtime {
    /// Probe the machine and load persisted cooldowns.
    pub async fn start() -> Self {
        let avail = availability::probe_all().await;
        let cooldown_path = cooldown::default_path();
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(crate::envfile::lookup("JEV_ALLOW_CREDITS").as_deref(), &avail);
        let mut runtime = Self::build(avail, cooldowns, allow, Overrides::from_env(), cooldown_path);
        runtime.seed_usage(crate::planstore::default_path());
        runtime
    }

    /// Fill in what is known about each provider's usage before any turn: the readings an
    /// earlier run saved, Codex's own latest reading from its session files (fresher than
    /// anything saved), and DeepSeek's balance in the background. From now on every new
    /// reading is saved to `plan_path`.
    fn seed_usage(&mut self, plan_path: Option<PathBuf>) {
        if let Some(path) = &plan_path {
            self.plan = crate::planstore::load(path);
        }
        self.plan_path = plan_path;
        self.refresh_codex_plan();
        self.refresh_balance();
    }

    /// Codex reports its limits only in its session files, not in the turn's output.
    fn refresh_codex_plan(&mut self) {
        if let Some((plan, _)) = crate::backend::codex_plan::latest_plan() {
            self.note_plan(ProviderId::Codex, plan);
        }
    }

    /// Fetch DeepSeek's balance off the prompt's path when it has a key. The answer lands
    /// in `balance` whenever it arrives; until then (or on any failure) none is shown.
    fn refresh_balance(&self) {
        let Some(key) = crate::envfile::lookup("DEEPSEEK_API_KEY").filter(|k| !k.trim().is_empty()) else { return };
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let slot = self.balance.clone();
        handle.spawn(async move {
            if let Some(b) = crate::balance::fetch_deepseek_balance(&key).await {
                *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(b);
            }
        });
    }

    /// Update what a finished turn may have changed: Codex's plan reading, DeepSeek's balance.
    pub fn after_turn(&mut self, provider: ProviderId) {
        match provider {
            ProviderId::Codex => self.refresh_codex_plan(),
            ProviderId::DeepSeek => self.refresh_balance(),
            _ => {}
        }
    }

    /// DeepSeek's remaining balance for display, `$12.34 left`, once the fetch has answered.
    pub fn credit_left(&self, p: ProviderId) -> Option<String> {
        if p != ProviderId::DeepSeek {
            return None;
        }
        self.balance.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(crate::balance::format_balance)
    }

    /// Like [`Runtime::start`], but a recent probe answer saved by an earlier run is used at
    /// once and the provider CLIs are asked again in the background; [`Runtime::absorb_late`]
    /// then corrects whatever changed. Without a usable cache the probe runs now, as in
    /// `start`, and its answer is saved for the next run. Says where the answer came from.
    pub async fn start_cached(cache: Option<ProbeCache>) -> (Self, ProbeSource) {
        let now = crate::log::now_ms();
        let hit = cache.as_ref().and_then(|c| probecache::load(&c.path, now, c.fingerprint));
        let Some(cached) = hit else {
            let runtime = Self::start().await;
            if let Some(c) = &cache {
                probecache::save(&c.path, crate::log::now_ms(), c.fingerprint, &runtime.avail);
            }
            return (runtime, ProbeSource::Live);
        };
        let cooldown_path = cooldown::default_path();
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(crate::envfile::lookup("JEV_ALLOW_CREDITS").as_deref(), &cached);
        let mut runtime = Self::build(cached.clone(), cooldowns, allow, Overrides::from_env(), cooldown_path);
        runtime.seed_usage(crate::planstore::default_path());
        runtime.refresh_in_background(cached, cache);
        (runtime, ProbeSource::Cache)
    }

    /// Ask every provider again, off the prompt's path. Each answer reaches
    /// [`Runtime::absorb_late`] as it arrives; the cache is rewritten when all are in.
    fn refresh_in_background(&mut self, cached: Vec<Availability>, cache: Option<ProbeCache>) {
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(|_| {}) });
        tokio::spawn(async move {
            let mut latest = cached;
            availability::retry_unknown(ALL_PROVIDERS.to_vec(), availability::PROBE_CAP, availability::probe_provider, |answer| {
                availability::merge_refresh(&mut latest, answer.clone());
                let _ = tx.send(answer);
            })
            .await;
            if let Some(c) = cache {
                probecache::save(&c.path, crate::log::now_ms(), c.fingerprint, &latest);
            }
        });
    }

    /// The one place a `Runtime` is assembled, so `start` and the tests share it.
    fn build(
        avail: Vec<Availability>,
        cooldowns: Cooldowns,
        allow: Allowances,
        overrides: Overrides,
        cooldown_path: Option<PathBuf>,
    ) -> Self {
        Self {
            avail,
            cooldowns,
            allow,
            overrides,
            cooldown_path,
            disk: Mutex::new(DiskWatch::default()),
            unusable_models: HashSet::new(),
            plan: HashMap::new(),
            plan_path: None,
            balance: std::sync::Arc::default(),
            blocked: HashMap::new(),
            late: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(avail: Vec<Availability>) -> Self {
        let allow = allowances_for(None, &avail);
        Self::build(avail, Cooldowns::default(), allow, Overrides::default(), None)
    }

    /// This process's cooldowns plus whatever other processes saved to the file. The
    /// file is only re-read when it changed (one `stat` otherwise); with no file this
    /// is just the in-memory set.
    fn current_cooldowns(&self) -> Cow<'_, Cooldowns> {
        let Some(path) = &self.cooldown_path else { return Cow::Borrowed(&self.cooldowns) };
        let mut disk = self.disk.lock().unwrap_or_else(PoisonError::into_inner);
        Cow::Owned(self.cooldowns.union(disk.refresh(path)))
    }

    /// The (provider, model) pairs routing may pick right now.
    pub fn usable(&self, now_ms: u64) -> Usable {
        let cooldowns = self.current_cooldowns();
        if self.blocked.is_empty() {
            return usable_from(&self.avail, &cooldowns, &self.unusable_models, now_ms);
        }
        let unblocked: Vec<Availability> =
            self.avail.iter().filter(|a| !self.blocked.contains_key(&a.provider)).cloned().collect();
        usable_from(&unblocked, &cooldowns, &self.unusable_models, now_ms)
    }

    /// How long `p` is still cooling down after a usage limit, in ms; `None` when it is not.
    pub fn cooldown_left(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.current_cooldowns().active(p, now_ms)
    }

    /// Skip `p` until its limit resets, and remember that across runs and processes:
    /// the save merges with the file, and the merged result (which includes limits
    /// other processes saved) becomes this process's set. A failed save costs only the
    /// persistence, so it is reported and otherwise ignored.
    pub fn cool_down(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.cooldowns.start(p, resets_at, now_ms);
        let Some(path) = &self.cooldown_path else { return };
        match self.cooldowns.save(path, now_ms) {
            Ok(merged) => self.cooldowns = merged,
            Err(e) => eprintln!("could not save cooldowns to {}: {e}", path.display()),
        }
    }

    /// Stop offering `model` of `p` for the rest of this run.
    pub fn mark_unusable(&mut self, p: ProviderId, model: &str) {
        self.unusable_models.insert((p, model.to_string()));
    }

    pub fn note_plan(&mut self, p: ProviderId, plan: PlanUsage) {
        if self.plan.get(&p) == Some(&plan) {
            return;
        }
        self.plan.insert(p, plan);
        if let Some(path) = &self.plan_path {
            crate::planstore::save(path, &self.plan);
        }
    }

    /// Fraction of the plan window used (as the CLI reported it), if a reading exists and
    /// its window has not reset since.
    pub fn plan_utilization(&self, p: ProviderId) -> Option<f64> {
        let now_s = crate::log::now_ms() / 1000;
        self.plan.get(&p).filter(|u| crate::planstore::is_current(u, now_s)).map(|u| u.utilization)
    }

    /// Apply what a failed attempt teaches the router and say what happens next. `Some`
    /// is the note for "try the next model"; `None` means the failure would likely recur
    /// on every provider, so the caller stops. Shared by a turn's failover
    /// ([`crate::turn::Engine`]) and by orchestration workers
    /// ([`crate::delegate::Delegator`]) so a limit, a login or an entitlement is handled
    /// one way everywhere.
    pub fn react(
        &mut self,
        provider: ProviderId,
        model: &str,
        err: &TurnError,
        excluded: &mut HashSet<(ProviderId, String)>,
        now: u64,
    ) -> Option<String> {
        match &err.kind {
            ErrorKind::Quota { resets_at } => {
                self.cool_down(provider, resets_at.as_deref(), now);
                let when = resets_at.as_deref().map(|r| format!(" (resets {r})")).unwrap_or_default();
                Some(format!("{provider} hit its limit{when}: trying the next model"))
            }
            ErrorKind::NotEntitled => {
                self.mark_unusable(provider, model);
                Some(format!("{provider}:{model} is not on your plan: trying the next model"))
            }
            ErrorKind::Auth => {
                self.blocked.insert(provider, BlockReason::Auth);
                Some(format!(
                    "{provider} is not logged in or its login expired (log in to {} again, then /providers refresh): trying the next model",
                    login_target(provider)
                ))
            }
            ErrorKind::UnusableHere => {
                self.blocked.insert(provider, BlockReason::UnusableHere);
                Some(format!("{provider} {}: trying the next model", unusable_here_hint(provider)))
            }
            ErrorKind::Stalled | ErrorKind::Empty => {
                excluded.insert((provider, model.to_string()));
                let what = if err.kind == ErrorKind::Stalled { "stopped answering" } else { "gave an empty answer" };
                Some(format!("{provider}:{model} {what}: trying the next model"))
            }
            ErrorKind::Other => None,
        }
    }

    /// Probe again (for `/providers refresh`). Cooldowns, unusable models and plan
    /// readings are kept: a re-login does not end a usage limit. A provider that failed
    /// its login is offered again once the probe sees it logged in. The per-token
    /// allowance is recomputed because a key may have been added.
    pub async fn refresh(&mut self) {
        self.apply_probe(availability::probe_all().await);
    }

    /// The pure half of [`Runtime::refresh`].
    fn apply_probe(&mut self, avail: Vec<Availability>) {
        self.avail = avail;
        self.allow = allowances_for(crate::envfile::lookup("JEV_ALLOW_CREDITS").as_deref(), &self.avail);
        let avail = &self.avail;
        self.blocked.retain(|p, reason| {
            *reason != BlockReason::Auth || !avail.iter().any(|a| a.provider == *p && a.state == State::Ready)
        });
    }

    /// Re-check once, in the background, the providers whose startup probe could not tell
    /// (a lazy wrapper's first run can outlast the startup cap), with a longer cap. Startup
    /// does not wait for it. The answers reach this runtime through [`Runtime::absorb_late`],
    /// where the caller holds `&mut self`, so a running turn never sees routing change under it.
    /// `on_ready` runs there too, once per provider that was Unknown and is now Ready: a
    /// late answer a `/providers refresh` already settled is refused, and so is not announced.
    pub fn retry_slow_probes(&mut self, on_ready: impl Fn(&Availability) + Send + Sync + 'static) {
        self.retry_slow_probes_with(availability::RETRY_CAP, availability::probe_provider, on_ready);
    }

    fn retry_slow_probes_with<P, Fut>(&mut self, cap: std::time::Duration, probe: P, on_ready: impl Fn(&Availability) + Send + Sync + 'static)
    where
        P: Fn(ProviderId) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Availability> + Send + 'static,
    {
        let providers = availability::retry_candidates(&self.avail);
        if providers.is_empty() {
            // A background refresh may be running; the caller still wants to hear what it finds.
            if let Some(late) = self.late.as_mut() {
                late.on_ready = Box::new(on_ready);
            }
            return;
        }
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::FillUnknown, on_ready: Box::new(on_ready) });
        tokio::spawn(availability::retry_unknown(providers, cap, probe, move |answer| {
            let _ = tx.send(answer);
        }));
    }

    /// Apply what the background retry has found so far, announce (through the `on_ready`
    /// given to [`Runtime::retry_slow_probes`]) each provider that became Ready by it, and
    /// return whether anything changed. Call it where no turn is running (before a command
    /// or a turn starts).
    pub fn absorb_late(&mut self) -> bool {
        use tokio::sync::mpsc::error::TryRecvError;
        let Some(late) = self.late.as_mut() else { return false };
        let merge = late.merge;
        let mut merged = self.avail.clone();
        let mut applied = false;
        let mut now_ready = Vec::new();
        let finished = loop {
            match late.answers.try_recv() {
                Ok(answer) => {
                    let was_ready = merged.iter().any(|a| a.provider == answer.provider && a.state == State::Ready);
                    let changed = match merge {
                        LateMerge::FillUnknown => availability::merge_late(&mut merged, answer.clone()),
                        LateMerge::Refresh => availability::merge_refresh(&mut merged, answer.clone()),
                    };
                    applied |= changed;
                    if changed && answer.state == State::Ready && !was_ready {
                        now_ready.push(answer);
                    }
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if applied {
            self.apply_probe(merged);
        }
        if let Some(late) = &self.late {
            now_ready.iter().for_each(|a| (late.on_ready)(a));
        }
        if finished {
            self.late = None;
        }
        applied
    }

    /// `/new`: the directory-dependent blocks end with the session they were learned in.
    pub fn forget_unusable_here(&mut self) {
        self.blocked.retain(|_, reason| *reason != BlockReason::UnusableHere);
    }

    /// The text of `/providers`: probe results, then what has changed since.
    pub fn render(&self, now_ms: u64) -> String {
        let mut out = vec![availability::render(&self.avail)];
        out.extend(self.cooldown_lines(now_ms));
        out.extend(self.unusable_lines());
        out.extend(self.blocked_lines());
        out.push(format!(
            "allowed: credits {}, per-token {}",
            yes_no(self.allow.credits),
            yes_no(self.allow.per_token)
        ));
        out.extend(self.overrides.notes.iter().map(|n| format!("override: {n}")));
        out.join("\n")
    }

    /// The compact startup view: only providers that are ready, any cooldowns carried
    /// over from earlier runs, and where to find the rest.
    pub fn render_summary(&self, now_ms: u64) -> String {
        let mut lines: Vec<String> = Vec::new();
        let ready = availability::render_ready(&self.avail);
        lines.push(if ready.is_empty() { "no provider is ready".to_string() } else { ready });
        lines.extend(self.cooldown_lines(now_ms));
        lines.push("/providers for details".to_string());
        lines.join("\n")
    }

    fn cooldown_lines(&self, now_ms: u64) -> Vec<String> {
        let cooldowns = self.current_cooldowns();
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| Some(format!("{p} cooling for {}", format_left(cooldowns.active(*p, now_ms)?))))
            .collect()
    }

    /// One line per provider blocked for this session, with why and what clears it.
    fn blocked_lines(&self) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| {
                let line = match self.blocked.get(p)? {
                    BlockReason::Auth => format!(
                        "{p} blocked this session: not logged in (log in to {}, then /providers refresh)",
                        login_target(*p)
                    ),
                    BlockReason::UnusableHere => {
                        format!("{p} blocked this session: {} (/new clears it)", unusable_here_hint(*p))
                    }
                };
                Some(line)
            })
            .collect()
    }

    fn unusable_lines(&self) -> Vec<String> {
        let mut models: Vec<_> = self.unusable_models.iter().collect();
        models.sort();
        models.into_iter().map(|(p, m)| format!("{p}:{m} refused by the provider, not offered")).collect()
    }
}

/// The tool a user logs in to again for `provider`.
pub fn login_target(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Gemini => "gemini (or agy)",
        ProviderId::Grok => "grok",
        ProviderId::DeepSeek => "DeepSeek: check DEEPSEEK_API_KEY",
    }
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini};

    const NOW: u64 = 1_760_000_000_000;

    #[test]
    fn a_run_nobody_can_answer_never_uses_the_host_prompt_protocol() {
        let req = crate::backend::TurnRequest { dir: "/tmp".into(), model: "m".into(), prompt: "hi".into(), images: vec![], session_id: None, mode: Default::default() };
        let args = |b: &dyn Backend| -> Vec<String> { b.command(&req).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        for provider in [Claude, DeepSeek] {
            // `-p` or piped input: plain `claude -p`, allowed tools and permission mode decide.
            let plain = run_backends(false)(provider);
            let a = args(&*plain);
            assert!(!plain.interactive(), "{provider}");
            assert!(!a.iter().any(|x| x == "--permission-prompt-tool" || x == crate::backend::claude::SESSION_NOTE), "{provider}: {a:?}");
            assert!(a.iter().any(|x| x == "--permission-mode"), "{provider}: {a:?}");
            // A session on a terminal asks the person.
            let asking = run_backends(true)(provider);
            let a = args(&*asking);
            assert!(asking.interactive(), "{provider}");
            assert!(a.iter().any(|x| x == "--permission-prompt-tool") && a.iter().any(|x| x == crate::backend::claude::SESSION_NOTE), "{provider}: {a:?}");
        }
    }

    fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    fn some_model(p: ProviderId) -> &'static catalog::ModelSpec {
        catalog::models().iter().find(|m| m.provider == p).expect("catalog has the provider")
    }

    fn runtime() -> Runtime {
        Runtime::for_tests(vec![
            availability(Claude, State::Ready),
            availability(Codex, State::Ready),
            availability(Gemini, State::NotLoggedIn),
            availability(DeepSeek, State::NoKey),
        ])
    }

    #[test]
    fn usable_has_ready_providers_only() {
        let u = runtime().usable(NOW);
        assert!(u.contains(some_model(Claude)));
        assert!(u.contains(some_model(Codex)));
        assert!(!u.contains(some_model(Gemini)));
        assert!(!u.contains(some_model(DeepSeek)));
    }

    #[test]
    fn usable_excludes_cooling_provider_until_it_resets() {
        let mut rt = runtime();
        rt.cooldowns.start(Claude, None, NOW);
        assert!(!rt.usable(NOW + 1).contains(some_model(Claude)));
        assert!(rt.usable(NOW + 1).contains(some_model(Codex)));
        // Default cooldown is an hour; after that the provider is back.
        assert!(rt.usable(NOW + 2 * 60 * MINUTE_MS).contains(some_model(Claude)));
    }

    #[test]
    fn usable_excludes_a_refused_model_but_not_its_siblings() {
        let mut rt = runtime();
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        assert!(claude.len() >= 2, "test needs two Claude models");
        rt.mark_unusable(Claude, claude[0].id);
        let u = rt.usable(NOW);
        assert!(!u.contains(claude[0]));
        assert!(u.contains(claude[1]));
    }

    #[test]
    fn login_model_list_narrows_usable() {
        let mut a = availability(Claude, State::Ready);
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        a.models = Some(vec![claude[0].id.to_string()]);
        let u = Runtime::for_tests(vec![a]).usable(NOW);
        assert!(u.contains(claude[0]));
        assert!(!u.contains(claude[1]));
    }

    #[test]
    fn allowances_follow_credits_env_and_deepseek_key() {
        let ready = [availability(DeepSeek, State::Ready)];
        let no_key = [availability(DeepSeek, State::NoKey)];
        assert_eq!(allowances_for(None, &ready), Allowances { credits: false, per_token: true });
        assert_eq!(allowances_for(Some("1"), &no_key), Allowances { credits: true, per_token: false });
        assert!(allowances_for(Some("true"), &[]).credits);
        assert!(allowances_for(Some(" TRUE "), &[]).credits);
        assert!(!allowances_for(Some("0"), &[]).credits);
        assert!(!allowances_for(Some("yes"), &[]).credits);
        assert!(!allowances_for(Some(""), &[]).credits);
    }

    #[test]
    fn render_mentions_cooldown_refusals_allowances_and_overrides() {
        let mut rt = runtime();
        rt.cooldowns.start(Claude, None, NOW);
        rt.mark_unusable(Codex, "some-model");
        rt.overrides.notes.push("JEV_CHAINS ignored: boom".to_string());
        let text = rt.render(NOW + 12 * MINUTE_MS);
        assert!(text.contains("claude cooling for 48m"), "{text}");
        assert!(!text.contains("codex cooling"), "{text}");
        assert!(text.contains("codex:some-model refused"), "{text}");
        assert!(text.contains("credits no, per-token no"), "{text}");
        assert!(text.contains("override: JEV_CHAINS ignored: boom"), "{text}");
        assert!(text.contains(&availability::render(&rt.avail)), "{text}");
    }

    #[test]
    fn render_without_events_has_no_cooldown_lines() {
        assert!(!runtime().render(NOW).contains("cooling"));
    }

    #[test]
    fn format_left_picks_a_readable_unit() {
        assert_eq!(format_left(0), "<1m");
        assert_eq!(format_left(59_999), "<1m");
        assert_eq!(format_left(12 * MINUTE_MS), "12m");
        assert_eq!(format_left(125 * MINUTE_MS), "2h 5m");
        assert_eq!(format_left((2 * 1440 + 4 * 60) * MINUTE_MS), "2d 4h");
    }

    #[test]
    fn plan_utilization_reports_the_latest_reading() {
        let mut rt = runtime();
        assert_eq!(rt.plan_utilization(Claude), None);
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.4, resets_at: None });
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.7, resets_at: None });
        assert_eq!(rt.plan_utilization(Claude), Some(0.7));
        assert_eq!(rt.plan_utilization(Codex), None);
        // A window that has already reset says nothing about now.
        rt.note_plan(Codex, PlanUsage { window: "5h".into(), utilization: 0.9, resets_at: Some(1) });
        assert_eq!(rt.plan_utilization(Codex), None);
    }

    #[test]
    fn new_readings_are_saved_and_a_restart_loads_them() {
        let dir = std::env::temp_dir().join(format!("jev-plan-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plan-usage.json");
        let mut rt = runtime();
        rt.plan_path = Some(path.clone());
        rt.note_plan(Claude, PlanUsage { window: "five_hour".into(), utilization: 0.4, resets_at: None });
        assert_eq!(crate::planstore::load(&path).get(&Claude).map(|p| p.utilization), Some(0.4));
        // The same reading again does not rewrite the file.
        std::fs::remove_file(&path).unwrap();
        rt.note_plan(Claude, PlanUsage { window: "five_hour".into(), utilization: 0.4, resets_at: None });
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cool_down_persists_to_the_given_path() {
        let path = std::env::temp_dir().join(format!("jev-providers-test-{}.json", std::process::id()));
        let mut rt = runtime();
        rt.cooldown_path = Some(path.clone());
        rt.cool_down(Codex, Some("1760003600"), NOW);
        let loaded = Cooldowns::load(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.active(Codex, NOW), Some(3_600_000));
        assert!(rt.cooldowns.active(Codex, NOW).is_some());
    }

    fn shared_path(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jev-providers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cooldowns.json");
        (dir, path)
    }

    #[test]
    fn two_processes_see_each_others_cooldowns_without_restarting() {
        let (dir, path) = shared_path("two");
        let (mut a, mut b) = (runtime(), runtime());
        a.cooldown_path = Some(path.clone());
        b.cooldown_path = Some(path.clone());
        assert!(a.usable(NOW).contains(some_model(Claude)));

        a.cool_down(Codex, None, NOW);
        b.cool_down(Claude, None, NOW);

        // The file holds both, because B's save merged with A's.
        let on_disk = Cooldowns::load(&path);
        assert!(on_disk.active(Codex, NOW).is_some() && on_disk.active(Claude, NOW).is_some());
        // A never cooled Claude itself, yet stops offering it, and still skips Codex.
        let usable = a.usable(NOW + 1);
        assert!(!usable.contains(some_model(Claude)));
        assert!(!usable.contains(some_model(Codex)));
        // B's own set picked up A's Codex limit through the merge.
        assert!(b.cooldowns.active(Codex, NOW).is_some());
        assert!(!b.usable(NOW + 1).contains(some_model(Codex)));
        // Once the limits pass, both are back.
        assert!(a.usable(NOW + 2 * 60 * MINUTE_MS).contains(some_model(Claude)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usable_reads_the_cooldown_file_only_when_it_changed() {
        let (dir, path) = shared_path("mtime");
        let mut a = runtime();
        a.cooldown_path = Some(path.clone());
        let reloads = |rt: &Runtime| rt.disk.lock().unwrap().reloads();
        a.usable(NOW);
        a.usable(NOW);
        assert_eq!(reloads(&a), 0, "no file, nothing to read");

        let mut other = Cooldowns::default();
        other.start(Claude, None, NOW);
        other.save(&path, NOW).unwrap();
        assert!(!a.usable(NOW + 1).contains(some_model(Claude)));
        assert_eq!(reloads(&a), 1);
        for _ in 0..5 {
            a.usable(NOW + 1);
        }
        assert_eq!(reloads(&a), 1, "unchanged file: no reload");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_garbage_cooldown_file_is_tolerated_by_usable_and_cool_down() {
        let (dir, path) = shared_path("garbage");
        std::fs::write(&path, "{ definitely not json").unwrap();
        let mut a = runtime();
        a.cooldown_path = Some(path.clone());
        assert!(a.usable(NOW).contains(some_model(Claude)));
        a.cool_down(Claude, None, NOW);
        assert!(!a.usable(NOW + 1).contains(some_model(Claude)));
        assert!(Cooldowns::load(&path).active(Claude, NOW).is_some(), "rewritten as valid json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cool_down_without_a_path_still_cools() {
        let mut rt = runtime();
        rt.cool_down(Claude, None, NOW);
        assert!(rt.cooldowns.active(Claude, NOW).is_some());
    }

    #[test]
    fn gemini_cli_needs_both_binary_and_auth() {
        assert_eq!(choose_gemini_flavor(true, true), Flavor::GeminiCli);
        assert_eq!(choose_gemini_flavor(true, false), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, true), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, false), Flavor::Antigravity);
    }

    #[test]
    fn deepseek_env_points_at_deepseek_with_the_key_twice() {
        let env = deepseek_env("sk-test");
        assert!(env.contains(&("ANTHROPIC_BASE_URL".to_string(), "https://api.deepseek.com/anthropic".to_string())));
        assert!(env.contains(&("ANTHROPIC_AUTH_TOKEN".to_string(), "sk-test".to_string())));
        assert!(env.contains(&("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())));
        assert_eq!(env.len(), 3);
    }

    #[test]
    fn a_directory_refusal_blocks_the_provider_with_a_reason_until_new_session() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::UnusableHere, message: "Not inside a trusted directory".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("codex needs a git repository here") && note.contains("trying the next model"), "{note}");
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none(), "not a usage limit");
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        assert!(rt.render(1_000).contains("codex blocked this session: needs a git repository here"), "{}", rt.render(1_000));
        // A probe that sees codex logged in does not change the directory ...
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        // ... but a login block still clears while this one stays; /new clears this one.
        rt.blocked.insert(ProviderId::Claude, BlockReason::Auth);
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.len(), 1);
        rt.forget_unusable_here();
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }

    #[test]
    fn a_failed_login_is_session_only_and_clears_when_the_probe_sees_a_login() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::Auth, message: "401".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("log in"), "{note}");
        // not in the persisted cooldowns, but not offered either
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none());
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        // a probe that still says "not logged in" keeps it blocked ...
        let mut not_logged = ready(ProviderId::Codex);
        not_logged.state = State::NotLoggedIn;
        rt.apply_probe(vec![ready(ProviderId::Claude), not_logged]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::Auth));
        // ... and one that sees the login again offers it right away
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }

    #[test]
    fn gemini_env_picks_up_keys_from_either_source() {
        let lookup = |n: &str| (n == "GEMINI_API_KEY").then(|| "from-file".to_string());
        assert_eq!(gemini_env(&lookup), vec![("GEMINI_API_KEY".to_string(), "from-file".to_string())]);
        assert!(gemini_env(&|_| None).is_empty());
    }

    #[test]
    fn a_max_or_ultra_claude_plan_allows_fable_but_pro_does_not() {
        let claude = |plan: &str| Availability {
            provider: ProviderId::Claude,
            state: State::Ready,
            plan: Some(plan.into()),
            models: None,
            detail: String::new(),
            verified: true,
        };
        assert!(allowances_for(None, &[claude("max_5x")]).credits);
        assert!(allowances_for(None, &[claude("Ultra")]).credits);
        assert!(!allowances_for(None, &[claude("pro")]).credits);
        assert!(allowances_for(Some("1"), &[claude("pro")]).credits, "the env opt-in still works on Pro");
    }

    // ---- a slow probe is re-checked once, in the background

    fn slow_start() -> Runtime {
        Runtime::for_tests(vec![availability(Claude, State::Ready), availability(Codex, State::Unknown), availability(Gemini, State::NotLoggedIn)])
    }

    #[tokio::test]
    async fn a_provider_that_timed_out_at_startup_becomes_usable_once_its_retry_succeeds() {
        let mut rt = slow_start();
        assert!(!rt.usable(NOW).contains(some_model(Codex)), "unknown is not routed to");
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        // The fake CLI answers after 30 ms, well inside the retry cap.
        let probe = |p: ProviderId| async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            availability(p, State::Ready)
        };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, move |a| said_in.lock().unwrap().push(a.provider));
        assert!(!rt.absorb_late(), "nothing has arrived yet, and asking must not block");
        assert!(said.lock().unwrap().is_empty(), "no notice before the answer is applied");
        while rt.late.is_some() {
            rt.absorb_late();
            tokio::task::yield_now().await;
        }
        assert_eq!(*said.lock().unwrap(), vec![Codex], "announced once, when it was applied");
        assert_eq!(rt.avail.iter().find(|a| a.provider == Codex).map(|a| a.state), Some(State::Ready));
        assert!(rt.usable(NOW).contains(some_model(Codex)));
        assert_eq!(rt.avail.iter().find(|a| a.provider == Gemini).map(|a| a.state), Some(State::NotLoggedIn));
        assert!(!rt.absorb_late(), "applied once");
    }

    #[tokio::test]
    async fn a_background_refresh_corrects_a_cached_answer_and_announces_only_what_became_ready() {
        let mut rt = Runtime::for_tests(vec![availability(Claude, State::Ready), availability(Codex, State::NotLoggedIn), availability(Gemini, State::Ready)]);
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        rt.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(move |a| said_in.lock().unwrap().push(a.provider)) });
        let _ = tx.send(availability(Claude, State::Ready)); // same as cached: nothing to do
        let _ = tx.send(availability(Codex, State::Ready)); // logged in since the cache was written
        let _ = tx.send(availability(Gemini, State::NotLoggedIn)); // logged out since
        let _ = tx.send(availability(ProviderId::Grok, State::Unknown)); // could not tell: ignored
        assert!(rt.absorb_late());
        assert_eq!(*said.lock().unwrap(), vec![Codex], "only a provider that became ready is announced");
        let state = |rt: &Runtime, p| rt.avail.iter().find(|a| a.provider == p).map(|a| a.state);
        assert_eq!((state(&rt, Codex), state(&rt, Gemini)), (Some(State::Ready), Some(State::NotLoggedIn)));
        drop(tx);
        assert!(!rt.absorb_late());
        assert!(rt.late.is_none(), "the refresh ends with its channel");
    }

    #[tokio::test]
    async fn the_slow_probe_hook_attaches_to_a_running_refresh_instead_of_replacing_it() {
        let mut rt = Runtime::for_tests(vec![availability(Claude, State::Ready)]);
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        rt.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(|_| {}) });
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), |p| async move { availability(p, State::Ready) }, move |a| said_in.lock().unwrap().push(a.provider));
        let _ = tx.send(availability(Codex, State::Ready));
        rt.avail.push(availability(Codex, State::NotLoggedIn));
        rt.absorb_late();
        assert_eq!(*said.lock().unwrap(), vec![Codex]);
    }

    #[tokio::test]
    async fn a_retry_that_ends_unknown_again_changes_nothing_and_is_not_repeated() {
        let mut rt = slow_start();
        let probe = |p: ProviderId| async move { availability(p, State::Unknown) };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, |_| panic!("not ready"));
        while rt.late.is_some() {
            assert!(!rt.absorb_late());
            tokio::task::yield_now().await;
        }
        assert_eq!(rt.avail[1].state, State::Unknown);
        assert!(rt.late.is_none(), "one retry only: the receiver is gone");
    }

    #[tokio::test]
    async fn nothing_is_retried_when_every_probe_gave_an_answer() {
        let mut rt = runtime();
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), |p| async move { availability(p, State::Ready) }, |_| panic!("no retry"));
        assert!(rt.late.is_none());
    }

    #[tokio::test]
    async fn a_refresh_that_already_found_the_provider_is_not_overwritten_by_the_late_answer() {
        let mut rt = slow_start();
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        // The late probe answers Ready, but only after the test has let a refresh settle it.
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(Some(gate)));
        let probe = move |p: ProviderId| {
            let gate = gate.clone();
            async move {
                if let Some(g) = gate.lock().await.take() {
                    let _ = g.await;
                }
                availability(p, State::Ready)
            }
        };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, move |a| said_in.lock().unwrap().push(a.provider));
        rt.apply_probe(vec![availability(Claude, State::Ready), availability(Codex, State::NotLoggedIn)]);
        release.send(()).unwrap();
        while rt.late.is_some() {
            rt.absorb_late();
            tokio::task::yield_now().await;
        }
        assert_eq!(rt.avail[1].state, State::NotLoggedIn);
        assert!(!rt.usable(NOW).contains(some_model(Codex)), "the refused answer does not make it routable");
        assert!(said.lock().unwrap().is_empty(), "and nobody is told it is ready: {:?}", said.lock().unwrap());
    }
}
