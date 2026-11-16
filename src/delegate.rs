//! The worker side of cross-provider orchestration: a planner model calls a
//! `delegate` tool with a brief, and a cheaper WORKER model, possibly from another
//! provider, does that piece in its own git worktree. The planner then reviews the
//! returned diff and decides what to merge.
//!
//! **Jev makes every model decision.** Nothing here takes a model from the caller: the
//! tool has no model parameter and [`DelegateRequest`] has no model field, so anything a
//! planner says about models cannot reach this code. Code only
//! 1. filters to the models that are really usable (logged in, not cooling down, billing
//!    allowed: [`crate::providers::Runtime`], [`crate::catalog::Usable`]),
//! 2. applies hard safety rules ([`pick_worker`]: never Top tier, never Fable/Astra/
//!    Carbon, security work never to DeepSeek/Grok, code-writing work never below
//!    Everyday), and
//! 3. maps Jev's judgment of the brief (its task type, via the catalog's chains) to the
//!    first model that passes.
//!
//! A worktree starts from `HEAD`, so the planner's UNCOMMITTED edits are invisible to
//! workers: the tool description tells the planner to commit what a worker needs or put
//! it in the brief.
//!
//! Layout: [`pick_worker`] and the text helpers are pure; [`Worktrees`] is the only git
//! door ([`GitWorktrees`] real, tests fake it); [`Delegator`] sequences judge, pick,
//! worktree, run, retry, log; [`DelegateApi`] is what the MCP server sees.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::backend::{self, Event, ProviderId, TurnError, TurnOutcome, TurnRequest};
use crate::catalog::{
    allowed_for, chain_with, models, pick, Allowances, ModelSpec, Overrides, TaskType, Tier, Usable,
};
use crate::log;
use crate::providers::Runtime;
use crate::router::{ProviderWant, Router, CODE_EDIT_P, CODE_SECURITY_P};
use crate::turn::{idle_timeout_from_env, BackendFactory};
use crate::turnlog::turn_row;

/// A worker that fails in a way another provider can fix is retried this many times in
/// all (so: once more), on the next worker pick.
pub const MAX_WORKER_ATTEMPTS: u32 = 2;
/// Most characters of a diff handed back to the planner.
pub const MAX_DIFF_CHARS: usize = 20_000;
/// Most characters of the worker's final text handed back.
pub const MAX_SUMMARY_CHARS: usize = 4_000;
/// A provider whose plan is this much busier than the least busy candidate is passed
/// over for the next in Jev's chain: within the margin chain order wins (it is Jev's
/// quality judgment), beyond it the work is spread to protect the busy plan.
const SPREAD_SLACK: f64 = 0.10;
/// How much one worker already running counts as plan usage when spreading, so parallel
/// workers fan out across providers even before any provider reports its usage.
const LIVE_WORKER_LOAD: f64 = 0.15;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// --------------------------------------------------------------- picking a worker

/// What Jev said about a brief, reduced to what choosing a worker needs.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefSignals {
    pub task: TaskType,
    /// P(the brief is about security), independent of keywords.
    pub security_p: f64,
    /// P(the brief changes files or runs commands).
    pub edit_p: f64,
    /// The planner says no files change. Only a hint: it lowers the floor to Light only
    /// when Jev also says the brief is not an edit.
    pub read_only: bool,
}

impl BriefSignals {
    pub fn from_want(w: &ProviderWant, read_only: bool) -> Self {
        Self { task: w.task, security_p: w.security_p, edit_p: w.edit_p, read_only }
    }

    /// Jev was unavailable. A brief nobody judged could be security work, which has hard
    /// vendor rules, so it is treated as security (never DeepSeek/Grok) and as an edit.
    pub fn unjudged() -> Self {
        Self { task: TaskType::Security, security_p: 1.0, edit_p: 1.0, read_only: false }
    }
}

/// The task type that picks the chain. Security is a hard rule that overrides Jev's task
/// label, like in routing, but judged by Jev's `security_p` only: the keyword lists
/// are not used for workers because a brief is full of incidental words ("no unsafe
/// code") that tagged every worker security-sensitive.
pub fn worker_task(s: &BriefSignals) -> TaskType {
    if s.task == TaskType::Security || s.security_p >= CODE_SECURITY_P {
        TaskType::Security
    } else {
        s.task
    }
}

/// The lowest tier this worker may run on. Work that writes code is never below
/// Everyday; Light is for lookups the planner calls read-only AND Jev agrees do not edit.
pub fn worker_floor(s: &BriefSignals) -> Tier {
    if s.read_only && s.edit_p < CODE_EDIT_P {
        Tier::Light
    } else {
        Tier::Everyday
    }
}

/// Workers are never Top tier (that is the planner's), and never the models kept for
/// very hard work (Fable, Astra, Carbon) whatever their tier says.
fn worker_eligible(spec: &ModelSpec, floor: Tier) -> bool {
    spec.tier != Tier::Top && !spec.very_hard_only && spec.tier >= floor
}

/// Why no worker can take a brief; the caller says so instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoWorker {
    pub task: TaskType,
    pub floor: Tier,
}

impl std::fmt::Display for NoWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let security = if self.task == TaskType::Security { " (security work never goes to deepseek or grok)" } else { "" };
        write!(
            f,
            "no usable worker model for {} work at {:?} tier or above{security}; workers are never top-tier models. \
             Do this piece yourself.",
            self.task, self.floor
        )
    }
}

impl std::error::Error for NoWorker {}

/// Everything [`pick_worker`] reads about the world; the caller owns the IO.
pub struct WorkerView<'a> {
    pub usable: &'a Usable,
    pub allow: &'a Allowances,
    pub overrides: &'a Overrides,
    /// Fraction of the plan used, as the provider last reported it.
    pub plan_utilization: &'a dyn Fn(ProviderId) -> Option<f64>,
    /// Workers currently running on the provider.
    pub live: &'a dyn Fn(ProviderId) -> usize,
    /// Rough size of the brief, so a model whose window is too small is skipped.
    pub context_tokens: u64,
}

/// Models Jev's chain for the task lists that may be workers, then every other eligible
/// model, strongest tier first. The tail is only used when the chain has no usable
/// worker (the Security and Review chains are all Top tier).
fn candidate_lists(task: TaskType, floor: Tier, overrides: &Overrides) -> [Vec<&'static ModelSpec>; 2] {
    let ok = |s: &&'static ModelSpec| worker_eligible(s, floor) && allowed_for(task, s);
    let chain: Vec<_> = chain_with(task, false, overrides).into_iter().filter(ok).collect();
    let mut rest: Vec<_> = models().iter().filter(ok).collect();
    rest.sort_by_key(|s| std::cmp::Reverse(s.tier));
    [chain, rest]
}

/// Among serving models (best first), one per provider at its best, prefer the least
/// busy provider; but a provider within [`SPREAD_SLACK`] of the least busy keeps its
/// chain position.
fn spread(serving: &[&'static ModelSpec], view: &WorkerView) -> &'static ModelSpec {
    let mut firsts: Vec<&'static ModelSpec> = Vec::new();
    for s in serving {
        if !firsts.iter().any(|f| f.provider == s.provider) {
            firsts.push(s);
        }
    }
    let load = |s: &ModelSpec| (view.plan_utilization)(s.provider).unwrap_or(0.0) + LIVE_WORKER_LOAD * (view.live)(s.provider) as f64;
    let least = firsts.iter().map(|s| load(s)).fold(f64::INFINITY, f64::min);
    firsts.iter().copied().find(|s| load(s) <= least + SPREAD_SLACK).unwrap_or(firsts[0])
}

/// The worker model for a brief. The only inputs are Jev's signals and the live view of
/// what is usable: there is no way to ask for a particular model.
pub fn pick_worker(signals: &BriefSignals, view: &WorkerView) -> Result<&'static ModelSpec, NoWorker> {
    let task = worker_task(signals);
    let floor = worker_floor(signals);
    for list in candidate_lists(task, floor, view.overrides) {
        let serving: Vec<_> = list
            .into_iter()
            .filter(|s| pick(&[*s], task, view.usable, view.allow, view.context_tokens).is_some())
            .collect();
        if !serving.is_empty() {
            return Ok(spread(&serving, view));
        }
    }
    Err(NoWorker { task, floor })
}

// --------------------------------------------------------------- text helpers

/// The paths a unified diff touches (both sides of a rename), in order, once each.
pub fn patch_paths(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("diff --git a/") else { continue };
        let (old, new) = rest.split_once(" b/").unwrap_or((rest, rest));
        for p in [old, new] {
            if !out.iter().any(|o| o == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// `text` cut to `max` characters, with a note of how much was dropped.
pub fn truncate_with_note(text: &str, max: usize, what: &str) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[{what} truncated: {} more characters not shown]", total - max)
}

/// What a worker is told before the brief: where it may work and how to report.
pub fn worker_prompt(brief: &str, files: &[String]) -> String {
    let hints = if files.is_empty() {
        String::new()
    } else {
        format!("\n\nFiles that are probably involved (hints, not limits): {}", files.join(", "))
    };
    format!(
        "You are a worker on one piece of a larger task. Rules: work only inside the current directory (your own git \
         worktree) and do not read or write outside it; do not use the network; do not commit or touch git branches. \
         When you are done, reply with a short report: what you changed, and anything you could not do.\n\n\
         Task:\n{brief}{hints}"
    )
}

// --------------------------------------------------------------- worktrees

/// The only door to git. Real: [`GitWorktrees`]; tests use a fake.
pub trait Worktrees: Send + Sync {
    /// A fresh isolated working copy at the main tree's `HEAD`, named `id`.
    fn create(&self, id: &str) -> Result<PathBuf>;
    /// Everything the worker changed in `path` (new files included) as one patch.
    fn diff(&self, path: &Path) -> Result<String>;
    /// Apply `patch` to the MAIN working tree. On failure the tree is left as it was.
    fn apply(&self, patch: &str) -> Result<()>;
    fn remove(&self, path: &Path) -> Result<()>;
}

/// Run `git -C dir args`, optionally feeding `stdin`; stdout on success, a message with
/// git's own words on failure.
fn git(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git (is it installed?)")?;
    // A separate thread, so a big patch cannot deadlock against git printing before it has read it all.
    let feeder = stdin.zip(child.stdin.take()).map(|(text, mut pipe)| {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        })
    });
    let out = child.wait_with_output().context("waiting for git")?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {}", args.first().unwrap_or(&""), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Files and the index as they were, so a failed merge can put the tree back exactly.
/// `git apply --3way` writes conflict markers into the working tree and stages partial
/// results when it fails, which is not "leave the tree untouched".
struct Snapshot {
    index: Option<(PathBuf, Vec<u8>)>,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Snapshot {
    fn take(repo: &Path, index: Option<PathBuf>, paths: &[String]) -> Self {
        let safe = |p: &&String| Path::new(p).components().all(|c| matches!(c, Component::Normal(_)));
        let files = paths.iter().filter(safe).map(|p| repo.join(p)).map(|p| {
            let bytes = std::fs::read(&p).ok();
            (p, bytes)
        });
        Self {
            index: index.and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))),
            files: files.collect(),
        }
    }

    fn restore(&self) {
        for (path, bytes) in &self.files {
            match bytes {
                Some(b) => {
                    let _ = std::fs::write(path, b);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        if let Some((path, bytes)) = &self.index {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Real worktrees: `git worktree add --detach <root>/<id> HEAD`. Detached, so no branch
/// is created and nothing is left behind once the worktree is removed.
pub struct GitWorktrees {
    /// The top level of the repository: workers start there, and merges apply there.
    repo: PathBuf,
    root: PathBuf,
}

impl GitWorktrees {
    /// `dir` is any directory inside the repository; worktrees go under `root`
    /// (`<state dir>/worktrees`).
    pub fn new(dir: &Path, root: PathBuf) -> Result<Self> {
        let top = git(dir, &["rev-parse", "--show-toplevel"], None)
            .with_context(|| format!("{} is not inside a git repository, so workers cannot get worktrees", dir.display()))?;
        Ok(Self { repo: PathBuf::from(top.trim()), root })
    }

    fn index_path(&self) -> Option<PathBuf> {
        let rel = git(&self.repo, &["rev-parse", "--git-path", "index"], None).ok()?;
        Some(self.repo.join(rel.trim()))
    }
}

impl Worktrees for GitWorktrees {
    fn create(&self, id: &str) -> Result<PathBuf> {
        let path = self.root.join(id);
        std::fs::create_dir_all(&self.root).with_context(|| format!("create {}", self.root.display()))?;
        git(&self.repo, &["worktree", "add", "--detach", &path.to_string_lossy(), "HEAD"], None)?;
        Ok(path)
    }

    /// `git add -A` first so files the worker created show up; `--binary` so the patch
    /// applies for binary files too.
    fn diff(&self, path: &Path) -> Result<String> {
        git(path, &["add", "-A"], None)?;
        git(path, &["diff", "--cached", "--binary", "HEAD"], None)
    }

    fn apply(&self, patch: &str) -> Result<()> {
        let snapshot = Snapshot::take(&self.repo, self.index_path(), &patch_paths(patch));
        match git(&self.repo, &["apply", "--3way", "-"], Some(patch)) {
            Ok(_) => Ok(()),
            Err(e) => {
                snapshot.restore();
                Err(anyhow!("the worker's changes do not apply cleanly (conflict with your tree); nothing was changed. {e}"))
            }
        }
    }

    fn remove(&self, path: &Path) -> Result<()> {
        git(&self.repo, &["worktree", "remove", "--force", &path.to_string_lossy()], None).map(|_| ())
    }
}

// --------------------------------------------------------------- the delegation API

/// What the planner asks for. There is deliberately no model field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegateRequest {
    pub brief: String,
    /// Hints about where the work is; not limits.
    pub files: Vec<String>,
    pub read_only: bool,
}

/// What comes back to the planner.
#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub worker_id: String,
    pub provider: Option<ProviderId>,
    pub model: Option<String>,
    /// The worker's final text, truncated.
    pub summary: String,
    pub files_changed: Vec<String>,
    /// The worker's patch, truncated to [`MAX_DIFF_CHARS`] with a note.
    pub diff: String,
    pub ok: bool,
    pub error: Option<String>,
    /// What happened to earlier attempts (a limit hit, a refused model).
    pub retries: Vec<String>,
}

impl DelegateResult {
    pub fn failed(error: String, retries: Vec<String>) -> Self {
        Self {
            worker_id: String::new(),
            provider: None,
            model: None,
            summary: String::new(),
            files_changed: vec![],
            diff: String::new(),
            ok: false,
            error: Some(error),
            retries,
        }
    }

    /// The text the planner reads.
    pub fn render(&self) -> String {
        let who = match (&self.provider, &self.model) {
            (Some(p), Some(m)) => format!("{p} {m}"),
            _ => "no worker".to_string(),
        };
        let mut out = vec![format!("worker {} ({who}): {}", self.worker_id, if self.ok { "done" } else { "FAILED" })];
        out.extend(self.retries.iter().map(|r| format!("retry: {r}")));
        if let Some(e) = &self.error {
            out.push(format!("error: {e}"));
        }
        if self.ok {
            out.push(format!("files changed: {}", if self.files_changed.is_empty() { "none".into() } else { self.files_changed.join(", ") }));
            out.push(format!("worker report:\n{}", self.summary));
            out.push(format!("diff:\n{}", if self.diff.is_empty() { "(empty)" } else { &self.diff }));
            out.push(format!("Review the diff, then merge({0}) to apply it or discard({0}) to drop it.", self.worker_id));
        }
        out.join("\n")
    }
}

/// What the MCP server calls. Implemented by [`Delegator`]; tests use a fake.
pub trait DelegateApi: Send + Sync {
    fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult>;
    /// Apply the worker's diff to the main tree and remove its worktree.
    fn merge(&self, worker_id: &str) -> Result<String, String>;
    /// Remove the worker's worktree without applying anything.
    fn discard(&self, worker_id: &str) -> Result<String, String>;
}

// --------------------------------------------------------------- the delegator

/// Jev's judgment of a brief: the same single batched request routing uses.
pub type Judge = Box<dyn Fn(String) -> BoxFut<'static, Option<ProviderWant>> + Send + Sync>;
type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// A [`Judge`] that asks `router` (`Router::provider_want`, one HTTP request).
pub fn router_judge(router: Arc<Router>) -> Judge {
    Box::new(move |brief| {
        let router = router.clone();
        Box::pin(async move { router.provider_want(&brief, "", None).await })
    })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct LiveWorker {
    provider: ProviderId,
    path: PathBuf,
    /// Still running; a finished worker awaiting merge or discard is not load.
    running: bool,
}

pub struct Delegator {
    runtime: Mutex<Runtime>,
    judge: Judge,
    worktrees: Box<dyn Worktrees>,
    backend_factory: BackendFactory,
    live: Mutex<HashMap<String, LiveWorker>>,
    counter: AtomicU64,
    /// Makes worker ids unique across runs that share a worktree directory.
    run_tag: u64,
    now: Clock,
    idle_timeout: Duration,
    log_path: Option<String>,
    planner: String,
}

/// One worker attempt that failed, in the words the planner and the log need.
struct AttemptFailure {
    id: String,
    spec: &'static ModelSpec,
    err: TurnError,
}

impl Delegator {
    /// Real backends, clock and idle timeout; no log, unknown planner.
    pub fn new(runtime: Runtime, judge: Judge, worktrees: Box<dyn Worktrees>) -> Self {
        let now: Clock = Box::new(log::now_ms);
        Self {
            runtime: Mutex::new(runtime),
            judge,
            worktrees,
            backend_factory: Box::new(crate::providers::backend_for),
            live: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
            run_tag: now() / 1000 % 1_000_000,
            now,
            idle_timeout: idle_timeout_from_env(),
            log_path: None,
            planner: String::new(),
        }
    }

    pub fn with_backend_factory(mut self, f: BackendFactory) -> Self {
        self.backend_factory = f;
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

    /// Append one row per worker attempt to this file (`orchestration.jsonl`).
    pub fn with_log(mut self, path: Option<String>) -> Self {
        self.log_path = path;
        self
    }

    /// The planner's `provider:model`, for the log.
    pub fn with_planner(mut self, planner: String) -> Self {
        self.planner = planner;
        self
    }

    fn next_id(&self) -> String {
        format!("w{}-{}", self.run_tag, self.counter.fetch_add(1, Ordering::SeqCst))
    }

    /// The worker for a brief, from the live view. Lock order is runtime, then the registry.
    fn pick(&self, signals: &BriefSignals, excluded: &HashSet<(ProviderId, String)>, brief_len: usize) -> Result<&'static ModelSpec, NoWorker> {
        let runtime = lock(&self.runtime);
        let usable = runtime.usable((self.now)()).without(excluded);
        let live = |p: ProviderId| lock(&self.live).values().filter(|w| w.running && w.provider == p).count();
        let utilization = |p: ProviderId| runtime.plan_utilization(p);
        pick_worker(
            signals,
            &WorkerView {
                usable: &usable,
                allow: &runtime.allow,
                overrides: &runtime.overrides,
                plan_utilization: &utilization,
                live: &live,
                context_tokens: (brief_len / 4) as u64,
            },
        )
    }

    /// Run the worker CLI in its worktree. Plan usage it reports feeds the spreading.
    async fn run_worker(&self, spec: &'static ModelSpec, dir: &Path, prompt: String) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
        let backend = (self.backend_factory)(spec.provider);
        let req = TurnRequest { dir: dir.to_path_buf(), model: spec.id.to_string(), prompt, session_id: None };
        let mut tools: Vec<String> = Vec::new();
        let mut on_event = |e: &Event| match e {
            Event::Tool(summary) => tools.push(summary.clone()),
            Event::Plan(plan) => lock(&self.runtime).note_plan(spec.provider, plan.clone()),
            _ => {}
        };
        let result = backend::run_turn(&*backend, &req, self.idle_timeout, &mut on_event).await;
        (result, tools)
    }

    fn set_running(&self, id: &str, running: bool) {
        if let Some(w) = lock(&self.live).get_mut(id) {
            w.running = running;
        }
    }

    /// Remove a worker's worktree and forget it. A failed removal is reported but the
    /// worker is forgotten anyway: a worktree that cannot be removed is a leftover to
    /// clean by hand (`git worktree prune`), not a reason to keep tracking it.
    fn forget(&self, id: &str) -> Result<(), String> {
        let worker = lock(&self.live).remove(id).ok_or_else(|| format!("unknown worker id `{id}` (already merged or discarded?)"))?;
        self.worktrees
            .remove(&worker.path)
            .map_err(|e| format!("worker {id} forgotten, but its worktree {} was not removed: {e}", worker.path.display()))
    }

    fn log_attempt(&self, a: &AttemptLog) {
        let Some(path) = &self.log_path else { return };
        let reason = format!("delegated {} brief", a.task);
        let mut row = turn_row(a.spec.provider, a.spec.id, None, &reason, a.jev, a.result, a.wall_ms);
        if let Value::Object(o) = &mut row {
            o.insert("kind".into(), json!("delegate"));
            o.insert("planner".into(), json!(self.planner));
            o.insert("worker_id".into(), json!(a.id));
            o.insert("task".into(), json!(a.task.as_str()));
            o.insert("attempt".into(), json!(a.attempt));
            o.insert("read_only".into(), json!(a.read_only));
            o.insert("files_changed".into(), json!(a.files));
            o.insert("tool_calls".into(), json!(a.tools));
        }
        log::append(path, &row);
    }

    /// One attempt: pick, create the worktree, run. `Err` is the failure to react to.
    async fn attempt(
        &self,
        req: &DelegateRequest,
        signals: &BriefSignals,
        jev: &Value,
        excluded: &HashSet<(ProviderId, String)>,
        attempt: u32,
    ) -> Result<DelegateResult, AttemptOutcome> {
        let spec = self.pick(signals, excluded, req.brief.len()).map_err(|e| AttemptOutcome::Stop(e.to_string()))?;
        let id = self.next_id();
        let path = self.worktrees.create(&id).map_err(|e| AttemptOutcome::Stop(format!("could not create a worktree: {e:#}")))?;
        lock(&self.live).insert(id.clone(), LiveWorker { provider: spec.provider, path: path.clone(), running: true });
        let started = Instant::now();
        let (result, tools) = self.run_worker(spec, &path, worker_prompt(&req.brief, &req.files)).await;
        self.set_running(&id, false);
        let wall_ms = started.elapsed().as_millis() as u64;
        let task = worker_task(signals);
        let log = |result: &Result<TurnOutcome, TurnError>, files: &[String]| {
            self.log_attempt(&AttemptLog { spec, id: &id, task, attempt, read_only: req.read_only, jev, result, wall_ms, files, tools: &tools })
        };
        match result {
            Err(err) => {
                log(&Err(err.clone()), &[]);
                // A failed worker's partial work is never offered; its worktree goes.
                let _ = self.forget(&id);
                Err(AttemptOutcome::Failed(AttemptFailure { id, spec, err }))
            }
            Ok(out) => match self.worktrees.diff(&path) {
                Err(e) => {
                    log(&Ok(out), &[]);
                    let _ = self.forget(&id);
                    Err(AttemptOutcome::Stop(format!("worker {id} finished but its changes could not be read: {e:#}")))
                }
                Ok(diff) => {
                    let files = patch_paths(&diff);
                    log(&Ok(out.clone()), &files);
                    Ok(DelegateResult {
                        worker_id: id,
                        provider: Some(spec.provider),
                        model: Some(spec.id.to_string()),
                        summary: truncate_with_note(out.text.trim(), MAX_SUMMARY_CHARS, "report"),
                        files_changed: files,
                        diff: truncate_with_note(&diff, MAX_DIFF_CHARS, "diff"),
                        ok: true,
                        error: None,
                        retries: vec![],
                    })
                }
            },
        }
    }

    /// Judge the brief, then run workers until one succeeds or the attempts are used up.
    pub async fn delegate(&self, req: DelegateRequest) -> DelegateResult {
        let want = (self.judge)(req.brief.clone()).await;
        let signals = want.as_ref().map_or_else(BriefSignals::unjudged, |w| BriefSignals::from_want(w, req.read_only));
        let jev = want.map_or(Value::Null, |w| w.signals);
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        let mut retries: Vec<String> = Vec::new();
        for attempt in 1..=MAX_WORKER_ATTEMPTS {
            match self.attempt(&req, &signals, &jev, &excluded, attempt).await {
                Ok(mut done) => {
                    done.retries = retries;
                    return done;
                }
                Err(AttemptOutcome::Stop(message)) => return DelegateResult::failed(message, retries),
                Err(AttemptOutcome::Failed(f)) => {
                    let react = lock(&self.runtime).react(f.spec.provider, f.spec.id, &f.err, &mut excluded, (self.now)());
                    match react {
                        Some(note) if attempt < MAX_WORKER_ATTEMPTS => retries.push(note),
                        _ => {
                            let mut failed = DelegateResult::failed(format!("{} {} failed: {}", f.spec.provider, f.spec.id, f.err.message), retries);
                            failed.worker_id = f.id;
                            failed.provider = Some(f.spec.provider);
                            failed.model = Some(f.spec.id.to_string());
                            return failed;
                        }
                    }
                }
            }
        }
        DelegateResult::failed("no worker succeeded".into(), retries)
    }

    pub fn merge(&self, id: &str) -> Result<String, String> {
        let (path, running) = {
            let live = lock(&self.live);
            let w = live.get(id).ok_or_else(|| format!("unknown worker id `{id}` (already merged or discarded?)"))?;
            (w.path.clone(), w.running)
        };
        if running {
            return Err(format!("worker {id} is still running"));
        }
        let diff = self.worktrees.diff(&path).map_err(|e| format!("could not read worker {id}'s changes: {e:#}"))?;
        if diff.trim().is_empty() {
            self.forget(id)?;
            return Ok(format!("worker {id} changed nothing; nothing to merge"));
        }
        let files = patch_paths(&diff);
        self.worktrees.apply(&diff).map_err(|e| format!("{e:#}\nWorker {id}'s worktree is kept; discard({id}) it or merge by hand."))?;
        let removed = self.forget(id);
        let note = removed.err().map(|e| format!(" ({e})")).unwrap_or_default();
        Ok(format!("merged worker {id}: {}. Changes are in your working tree (staged by git's 3-way apply).{note}", files.join(", ")))
    }

    pub fn discard(&self, id: &str) -> Result<String, String> {
        self.forget(id).map(|()| format!("discarded worker {id}"))
    }
}

impl DelegateApi for Delegator {
    fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult> {
        Box::pin(Delegator::delegate(self, req))
    }

    fn merge(&self, worker_id: &str) -> Result<String, String> {
        Delegator::merge(self, worker_id)
    }

    fn discard(&self, worker_id: &str) -> Result<String, String> {
        Delegator::discard(self, worker_id)
    }
}

/// How one attempt ended without a result.
enum AttemptOutcome {
    /// Nothing more to try (no worker, no worktree, unreadable changes).
    Stop(String),
    /// The worker failed; [`Runtime::react`] says whether another model may try.
    Failed(AttemptFailure),
}

/// Everything one log row needs.
struct AttemptLog<'a> {
    spec: &'static ModelSpec,
    id: &'a str,
    task: TaskType,
    attempt: u32,
    read_only: bool,
    jev: &'a Value,
    result: &'a Result<TurnOutcome, TurnError>,
    wall_ms: u64,
    files: &'a [String],
    tools: &'a [String],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use tokio::process::Command as TokioCommand;

    use crate::availability::{Availability, State};
    use crate::backend::{classify_common, Backend, ErrorKind, Finish, Usage};
    use crate::router::Security;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];
    const NOW: u64 = 1_760_000_000_000;

    fn providers_in(mask: u32) -> Vec<ProviderId> {
        PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect()
    }

    fn signals(task: TaskType) -> BriefSignals {
        BriefSignals { task, security_p: 0.0, edit_p: 0.9, read_only: false }
    }

    struct World {
        usable: Usable,
        allow: Allowances,
        overrides: Overrides,
        util: HashMap<ProviderId, f64>,
        live: HashMap<ProviderId, usize>,
    }

    impl World {
        fn new(ready: &[ProviderId]) -> Self {
            Self {
                usable: Usable::all_of(ready.iter().copied()),
                allow: Allowances { credits: true, per_token: true },
                overrides: Overrides::default(),
                util: HashMap::new(),
                live: HashMap::new(),
            }
        }

        fn pick(&self, s: &BriefSignals) -> Result<&'static ModelSpec, NoWorker> {
            let util = |p: ProviderId| self.util.get(&p).copied();
            let live = |p: ProviderId| self.live.get(&p).copied().unwrap_or(0);
            pick_worker(
                s,
                &WorkerView {
                    usable: &self.usable,
                    allow: &self.allow,
                    overrides: &self.overrides,
                    plan_utilization: &util,
                    live: &live,
                    context_tokens: 0,
                },
            )
        }
    }

    #[test]
    fn a_worker_is_never_top_tier_never_a_very_hard_model_and_always_usable() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let ready = providers_in(mask);
            let world = World::new(&ready);
            for task in TaskType::ALL {
                for read_only in [false, true] {
                    let s = BriefSignals { task, security_p: 0.1, edit_p: 0.1, read_only };
                    if let Ok(spec) = world.pick(&s) {
                        assert_ne!(spec.tier, Tier::Top, "{task} {ready:?}: {}", spec.id);
                        assert!(!spec.very_hard_only, "{}", spec.id);
                        assert!(ready.contains(&spec.provider), "unusable provider {} for {ready:?}", spec.provider);
                    }
                }
            }
        }
    }

    #[test]
    fn security_briefs_never_go_to_deepseek_or_grok() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let world = World::new(&providers_in(mask));
            // Jev's task label, and Jev's security probability on an otherwise ordinary brief.
            for s in [
                signals(TaskType::Security),
                BriefSignals { security_p: 0.95, ..signals(TaskType::Feature) },
                BriefSignals { security_p: 0.95, read_only: true, edit_p: 0.0, ..signals(TaskType::Explain) },
                BriefSignals::unjudged(),
            ] {
                if let Ok(spec) = world.pick(&s) {
                    assert!(!matches!(spec.provider, DeepSeek | Grok), "{:?} -> {} {}", s, spec.provider, spec.id);
                }
            }
        }
        // With only the barred providers ready there is no worker, rather than a bad one.
        let only_barred = World::new(&[DeepSeek, Grok]);
        assert_eq!(only_barred.pick(&signals(TaskType::Security)), Err(NoWorker { task: TaskType::Security, floor: Tier::Everyday }));
    }

    #[test]
    fn code_never_drops_below_everyday_but_a_read_only_lookup_may_use_light() {
        let world = World::new(&PROVIDERS);
        for task in TaskType::ALL {
            let writes = world.pick(&signals(task)).unwrap();
            assert!(writes.tier >= Tier::Everyday, "{task}: {}", writes.id);
            // The planner calling it read-only is not enough when Jev says it edits.
            let claims = BriefSignals { read_only: true, edit_p: 0.9, ..signals(task) };
            assert!(world.pick(&claims).unwrap().tier >= Tier::Everyday, "{task}");
        }
        let lookup = BriefSignals { read_only: true, edit_p: 0.05, ..signals(TaskType::Explain) };
        assert_eq!(world.pick(&lookup).unwrap().tier, Tier::Light, "Explain's chain starts with a Light model");
    }

    #[test]
    fn an_all_top_chain_falls_back_to_the_next_best_worker() {
        // Review's chain is Top models only; a review brief still gets an Everyday worker.
        let world = World::new(&[Claude]);
        assert_eq!(world.pick(&signals(TaskType::Review)).unwrap().id, "claude-sonnet-5-5");
    }

    #[test]
    fn chain_order_wins_until_a_provider_is_much_busier() {
        let mut world = World::new(&[Claude, Codex]);
        let first = world.pick(&signals(TaskType::Feature)).unwrap();
        assert_eq!(first.provider, Claude, "Feature's chain starts with Sonnet");
        world.util.insert(Claude, 0.05);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Claude, "within the margin");
        world.util.insert(Claude, 0.60);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Codex, "claude's plan is much busier");
    }

    #[test]
    fn parallel_workers_fan_out_across_providers() {
        let mut world = World::new(&[Claude, Codex]);
        world.live.insert(Claude, 1);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Codex, "one worker already on claude");
        world.live.insert(Codex, 1);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Claude, "tied: chain order again");
    }

    #[test]
    fn spreading_never_picks_a_model_the_rules_bar() {
        // DeepSeek is idle and Claude is busy, but security still cannot use DeepSeek.
        let mut world = World::new(&[Claude, DeepSeek]);
        world.util.insert(Claude, 0.95);
        assert_eq!(world.pick(&signals(TaskType::Security)).unwrap().provider, Claude);
    }

    #[test]
    fn jev_unavailable_is_treated_as_security_and_as_an_edit() {
        let u = BriefSignals::unjudged();
        assert_eq!(worker_task(&u), TaskType::Security);
        assert_eq!(worker_floor(&u), Tier::Everyday);
    }

    #[test]
    fn patch_paths_lists_each_touched_file_once() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-x\n+y\n\
                     diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n\
                     diff --git a/src/a.rs b/src/a.rs\n";
        assert_eq!(patch_paths(patch), ["src/a.rs", "old.txt", "new.txt"]);
        assert!(patch_paths("").is_empty());
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(truncate_with_note("short", 10, "diff"), "short");
        let cut = truncate_with_note(&"x".repeat(30), 10, "diff");
        assert!(cut.starts_with(&"x".repeat(10)) && cut.contains("diff truncated: 20 more characters"), "{cut}");
    }

    #[test]
    fn the_worker_prompt_confines_the_worker_and_asks_for_a_short_report() {
        let p = worker_prompt("add a retry", &["src/net.rs".into()]);
        assert!(p.contains("only inside the current directory") && p.contains("do not use the network"));
        assert!(p.contains("short report") && p.contains("add a retry") && p.contains("src/net.rs"));
        assert!(!worker_prompt("x", &[]).contains("Files that are probably involved"));
    }

    // ---------------------------------------------------------------- the Delegator

    /// A "CLI" that is a shell script with canned lines: `FIN:text`, `FAIL:message`.
    struct Fake {
        provider: ProviderId,
        script: &'static str,
        started: Arc<Mutex<Vec<(ProviderId, String, PathBuf, String)>>>,
    }

    impl Backend for Fake {
        fn provider(&self) -> ProviderId {
            self.provider
        }
        fn command(&self, req: &TurnRequest) -> TokioCommand {
            self.started.lock().unwrap().push((self.provider, req.model.clone(), req.dir.clone(), req.prompt.clone()));
            let mut c = TokioCommand::new("sh");
            c.arg("-c").arg(self.script).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            let line = line.trim_end();
            match line.split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish {
                    text: t.into(),
                    session_id: None,
                    usage: Usage { output_tokens: 7, ..Usage::default() },
                })],
                Some(("FAIL", m)) => vec![Event::Failed { message: m.into() }],
                Some(("TOOL", t)) => vec![Event::Tool(t.into())],
                _ => vec![],
            }
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            classify_common(message).unwrap_or(ErrorKind::Other)
        }
    }

    const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:did the piece\\n'";
    const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";

    /// Directories made and removed, and what `apply` was given; no git involved.
    #[derive(Default)]
    struct FakeWorktrees {
        base: PathBuf,
        created: Mutex<Vec<PathBuf>>,
        removed: Mutex<Vec<PathBuf>>,
        applied: Mutex<Vec<String>>,
        apply_fails: bool,
    }

    static UNIQUE: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = UNIQUE.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("jev-delegate-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    impl FakeWorktrees {
        fn new() -> Arc<Self> {
            Arc::new(Self { base: temp_dir("fake"), ..Self::default() })
        }
    }

    impl Worktrees for Arc<FakeWorktrees> {
        fn create(&self, id: &str) -> Result<PathBuf> {
            let p = self.base.join(id);
            std::fs::create_dir_all(&p)?;
            self.created.lock().unwrap().push(p.clone());
            Ok(p)
        }
        fn diff(&self, path: &Path) -> Result<String> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            Ok(format!("diff --git a/{name}.rs b/{name}.rs\n--- a/{name}.rs\n+++ b/{name}.rs\n@@ -1 +1 @@\n-a\n+b\n"))
        }
        fn apply(&self, patch: &str) -> Result<()> {
            if self.apply_fails {
                bail!("conflict");
            }
            self.applied.lock().unwrap().push(patch.to_string());
            Ok(())
        }
        fn remove(&self, path: &Path) -> Result<()> {
            self.removed.lock().unwrap().push(path.to_path_buf());
            let _ = std::fs::remove_dir_all(path);
            Ok(())
        }
    }

    fn want(task: TaskType) -> ProviderWant {
        ProviderWant {
            task,
            task_confidence: 0.9,
            very_hard_p: 0.0,
            security_p: 0.0,
            edit_p: 0.9,
            continues_p: 0.0,
            correction_p: None,
            multi_part_p: 0.0,
            security_keywords: Security::None,
            signals: json!({ "task": task.as_str() }),
        }
    }

    fn availability(provider: ProviderId, ready: bool) -> Availability {
        let state = if ready { State::Ready } else { State::NotLoggedIn };
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    struct Rig {
        d: Delegator,
        wt: Arc<FakeWorktrees>,
        started: Arc<Mutex<Vec<(ProviderId, String, PathBuf, String)>>>,
        scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>>,
    }

    fn rig(ready: &[ProviderId], task: TaskType) -> Rig {
        let avail = PROVIDERS.iter().map(|p| availability(*p, ready.contains(p))).collect();
        let wt = FakeWorktrees::new();
        let started = Arc::new(Mutex::new(Vec::new()));
        let scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>> =
            Arc::new(Mutex::new(PROVIDERS.iter().map(|p| (*p, OK)).collect()));
        let (st, sc) = (started.clone(), scripts.clone());
        let judge: Judge = Box::new(move |_| Box::pin(async move { Some(want(task)) }));
        let d = Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()))
            .with_backend_factory(Box::new(move |p| {
                let script = sc.lock().unwrap()[&p];
                Box::new(Fake { provider: p, script, started: st.clone() })
            }))
            .with_clock(Box::new(|| NOW))
            .with_idle_timeout(Duration::from_millis(500))
            .with_planner("claude:claude-opus-5-5".into());
        Rig { d, wt, started, scripts }
    }

    fn brief(text: &str) -> DelegateRequest {
        DelegateRequest { brief: text.into(), ..DelegateRequest::default() }
    }

    #[tokio::test]
    async fn a_worker_runs_in_its_own_worktree_and_reports_summary_and_diff() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        let res = r.d.delegate(DelegateRequest { files: vec!["src/net.rs".into()], ..brief("add a retry to the client") }).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.provider, Some(Claude));
        assert_eq!(res.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(res.summary, "did the piece");
        assert_eq!(res.files_changed, [format!("{}.rs", res.worker_id)]);
        assert!(res.diff.contains("+b"));
        let started = r.started.lock().unwrap().clone();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].2, r.wt.base.join(&res.worker_id), "the worker ran in its worktree");
        assert!(started[0].3.contains("add a retry to the client") && started[0].3.contains("src/net.rs"));
        assert!(res.render().contains(&format!("merge({})", res.worker_id)));
    }

    #[tokio::test]
    async fn two_delegations_get_two_separate_directories() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        let (a, b) = tokio::join!(r.d.delegate(brief("piece one")), r.d.delegate(brief("piece two")));
        assert!(a.ok && b.ok);
        assert_ne!(a.worker_id, b.worker_id);
        let dirs: HashSet<_> = r.started.lock().unwrap().iter().map(|s| s.2.clone()).collect();
        assert_eq!(dirs.len(), 2, "workers never share a directory");
        // Run together, the second pick sees the first as live and goes to the other provider.
        let providers: HashSet<_> = r.started.lock().unwrap().iter().map(|s| s.0).collect();
        assert_eq!(providers.len(), 2, "parallel workers spread across providers");
    }

    #[tokio::test]
    async fn quota_retries_once_on_a_different_provider_and_cools_the_first_down() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        let res = r.d.delegate(brief("add a retry")).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.provider, Some(Codex), "Sonnet hit its limit; the next worker is another provider");
        assert_eq!(res.model.as_deref(), Some("gpt-6.1-terra"));
        let started = r.started.lock().unwrap().clone();
        assert_eq!(started.iter().map(|s| s.0).collect::<Vec<_>>(), [Claude, Codex]);
        assert_ne!(started[0].1, started[1].1);
        assert_eq!(res.retries.len(), 1);
        assert!(res.retries[0].contains("hit its limit"), "{:?}", res.retries);
        assert!(lock(&r.d.runtime).cooldowns.active(Claude, NOW).is_some());
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1, "the failed worker's worktree is gone");
        assert_eq!(r.wt.created.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_second_failure_ends_the_delegation_instead_of_looping() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        r.scripts.lock().unwrap().insert(Codex, QUOTA);
        let res = r.d.delegate(brief("add a retry")).await;
        assert!(!res.ok);
        assert_eq!(r.started.lock().unwrap().len(), 2, "one retry, no more");
        assert!(res.error.unwrap().contains("failed"));
    }

    #[tokio::test]
    async fn an_unusable_provider_never_starts_and_no_worker_is_reported_plainly() {
        let r = rig(&[Grok, DeepSeek], TaskType::Security);
        let res = r.d.delegate(brief("audit the login route")).await;
        assert!(!res.ok);
        assert!(r.started.lock().unwrap().is_empty(), "nothing was started");
        assert!(res.error.unwrap().contains("no usable worker model"));
        // And a worker only ever starts on a ready provider.
        let r = rig(&[Codex], TaskType::Feature);
        r.d.delegate(brief("x")).await;
        assert!(r.started.lock().unwrap().iter().all(|s| s.0 == Codex));
    }

    #[tokio::test]
    async fn a_failure_no_other_provider_can_fix_stops_at_once() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, "printf 'FAIL:segmentation fault\\n'");
        let res = r.d.delegate(brief("x")).await;
        assert!(!res.ok);
        assert_eq!(r.started.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn merge_applies_the_workers_diff_and_removes_its_worktree() {
        let r = rig(&[Claude], TaskType::Feature);
        let res = r.d.delegate(brief("x")).await;
        let msg = r.d.merge(&res.worker_id).unwrap();
        assert!(msg.contains("merged worker") && msg.contains(&res.worker_id));
        assert_eq!(r.wt.applied.lock().unwrap().len(), 1);
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1);
        assert!(r.d.merge(&res.worker_id).unwrap_err().contains("unknown worker id"), "a worker merges once");
    }

    #[tokio::test]
    async fn a_failed_merge_keeps_the_worktree_for_discard() {
        let wt = Arc::new(FakeWorktrees { base: temp_dir("conflict"), apply_fails: true, ..FakeWorktrees::default() });
        let avail = PROVIDERS.iter().map(|p| availability(*p, *p == Claude)).collect();
        let judge: Judge = Box::new(|_| Box::pin(async { Some(want(TaskType::Feature)) }));
        let st = Arc::new(Mutex::new(Vec::new()));
        let d = Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()))
            .with_backend_factory(Box::new(move |p| Box::new(Fake { provider: p, script: OK, started: st.clone() })))
            .with_idle_timeout(Duration::from_millis(500));
        let res = d.delegate(brief("x")).await;
        let err = d.merge(&res.worker_id).unwrap_err();
        assert!(err.contains("conflict") && err.contains("discard"), "{err}");
        assert!(wt.removed.lock().unwrap().is_empty());
        assert!(d.discard(&res.worker_id).unwrap().contains("discarded"));
        assert_eq!(wt.removed.lock().unwrap().len(), 1);
        assert!(d.discard(&res.worker_id).is_err());
    }

    #[tokio::test]
    async fn each_attempt_is_logged_with_planner_worker_signals_and_files() {
        let log_path = temp_dir("log").join("orchestration.jsonl");
        let mut r = rig(&[Claude, Codex], TaskType::Feature);
        r.d = r.d.with_log(Some(log_path.to_string_lossy().into_owned()));
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        r.d.delegate(brief("add a retry")).await;
        let rows: Vec<Value> = std::fs::read_to_string(&log_path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "delegate");
        assert_eq!(rows[0]["planner"], "claude:claude-opus-5-5");
        assert_eq!((rows[0]["ok"].clone(), rows[0]["error_kind"].clone()), (json!(false), json!("quota")));
        assert_eq!(rows[1]["provider"], "codex");
        assert_eq!(rows[1]["ok"], true);
        assert_eq!(rows[1]["attempt"], 2);
        assert_eq!(rows[1]["signals"]["task"], "feature");
        assert_eq!(rows[1]["usage"]["output_tokens"], 7);
        assert_eq!(rows[1]["files_changed"].as_array().unwrap().len(), 1);
        assert!(rows[1]["wall_ms"].is_u64());
    }

    // ---------------------------------------------------------------- real git

    fn run_git(dir: &Path, args: &[&str]) -> String {
        git(dir, args, None).unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A repository with one commit holding `f.txt` (five lines).
    fn temp_repo() -> PathBuf {
        let d = temp_dir("repo");
        run_git(&d, &["init", "-q"]);
        std::fs::write(d.join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();
        run_git(&d, &["add", "."]);
        run_git(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        d
    }

    #[test]
    fn real_worktrees_are_isolated_and_diff_includes_new_files() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let (a, b) = (wt.create("wa").unwrap(), wt.create("wb").unwrap());
        assert_ne!(a, b);
        std::fs::write(a.join("f.txt"), "a\nB-from-a\nc\nd\ne\n").unwrap();
        std::fs::write(b.join("new.txt"), "hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(b.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n", "b does not see a's edit");
        assert!(!a.join("new.txt").exists(), "a does not see b's file");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n", "the main tree is untouched");
        let (da, db) = (wt.diff(&a).unwrap(), wt.diff(&b).unwrap());
        assert_eq!(patch_paths(&da), ["f.txt"]);
        assert_eq!(patch_paths(&db), ["new.txt"]);
        wt.remove(&a).unwrap();
        assert!(!a.exists() && b.exists());
    }

    #[test]
    fn real_merge_applies_a_workers_diff_to_the_main_tree() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        // The planner has since changed another part of the same file: a 3-way merge keeps both.
        std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        wt.apply(&wt.diff(&w).unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nB-from-worker\nc\nd\nE-from-planner\n");
        assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn a_conflicting_merge_leaves_the_tree_untouched() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        // The planner changed the very same line.
        std::fs::write(repo.join("f.txt"), "a\nB-from-planner\nc\nd\ne\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        let before_status = run_git(&repo, &["status", "--porcelain"]);
        let before_index = run_git(&repo, &["ls-files", "--stage"]);
        let err = wt.apply(&patch).unwrap_err();
        assert!(format!("{err:#}").contains("nothing was changed"), "{err:#}");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nB-from-planner\nc\nd\ne\n", "no conflict markers");
        assert!(!repo.join("new.txt").exists(), "no partial application");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), before_status);
        assert_eq!(run_git(&repo, &["ls-files", "--stage"]), before_index, "the index is as it was");
    }

    #[test]
    fn worktrees_need_a_git_repository() {
        let not_a_repo = temp_dir("plain");
        let err = GitWorktrees::new(&not_a_repo, temp_dir("root")).err().expect("not a repository");
        assert!(format!("{err:#}").contains("not inside a git repository"));
    }

    #[test]
    fn the_result_text_names_the_worker_and_failure() {
        let failed = DelegateResult::failed("boom".into(), vec!["claude hit its limit".into()]);
        let text = failed.render();
        assert!(text.contains("FAILED") && text.contains("error: boom") && text.contains("retry: claude hit its limit"));
    }
}
