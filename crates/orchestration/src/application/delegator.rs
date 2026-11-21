//! The worker side of cross-provider orchestration: a planner model calls a
//! `delegate` tool with a brief, and a cheaper WORKER model, possibly from another
//! provider, does that piece in its own git worktree. The planner then reviews the
//! returned diff and decides what to merge.
//!
//! **Jev makes every model decision.** Nothing here takes a model from the caller: the
//! tool has no model parameter and [`DelegateRequest`] has no model field, so anything a
//! planner says about models cannot reach this code. Code only
//! 1. filters to the models that are really usable (logged in, not cooling down, billing
//!    allowed: [`jev_providers::application::provider_registry::Runtime`], [`jev_catalog::domain::model_table::Usable`]),
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
//!
//! **Where a worker runs and how its work lands** ([`Landing`]). A read-only worker runs in
//! the user's own folder in plan mode, when its provider enforces plan mode
//! ([`plan_mode_enforced`]), and returns only a report; on a provider that does not, it runs
//! in a worktree that is thrown away after. An editing worker gets its own worktree; when it
//! finishes, jev runs the project's tests there ([`crate::infrastructure::worker_check`]) and, if they pass and
//! the changes apply cleanly, merges them itself: the planner gets the report and the file
//! list, not the diff, and `undo` takes them back out. When there is no test command, the
//! tests fail, the patch conflicts, or the planner asked to review, it falls back to review.
//!
//! **Delegations run in the background.** [`Delegator::start`] returns a task id at once and
//! runs the worker (and its tests) on its own task; [`Delegator::wait`] collects it and only
//! then lands it ([`Delegator::settle`]), so nothing reaches the main tree while the planner
//! may be editing it. [`Delegator::delegate`] does both in one blocking call.
//!
//! **In review, the planner merges only what it saw.** The delegate result lists every changed file
//! with its +/- counts and carries whole-file diffs up to a size budget; `show_diff` pages
//! the rest. The registry records which files were shown in full and `merge` refuses any
//! other. The registry is saved to `<state>/workers` so a restarted server still knows
//! its workers, and [`Delegator::recover`] removes abandoned checkouts. Git calls and
//! record IO are blocking, so the delegator runs them on the blocking pool
//! ([`Delegator::off_thread`]) and never under the registry lock.
//!
//! **Servers share the state directory.** Every record carries its owner (pid plus process
//! start time, [`Liveness`]); `recover` never touches a record whose owner is alive, and
//! adopts or ages out only a dead server's. Worker ids carry a per-server tag, so two
//! servers never produce the same id. A delegation dropped mid-flight (a blocking call
//! cancelled, or a background task dropped when the server shuts down) removes its
//! placeholder, record and worktree ([`Reservation`]). A path read from a record is
//! deleted only if it proves to be inside the worktrees root ([`checked_inside`]).

mod attempt;
mod background;
mod land;
mod outcome;
mod recover;
mod reservation;
mod review;
mod run;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{anyhow, Result};

use jev_backend::{BackendFactory, ProviderId};
use jev_catalog::domain::model_table::ModelSpec;
use crate::domain::brief_signals::BriefSignals;
use crate::domain::delegation::DelegateRequest;
use crate::domain::worker::{Owner, Saved, Worker};
use crate::domain::worker_pick::{pick_worker, NoWorker, WorkerView};
use jev_providers::application::provider_registry::Runtime;

use super::ports::{Adapters, BoxFut, Checks, DelegateApi, DelegationLog, Judge, Liveness, Records, Scanner, WorkerPolicy, Worktrees};
use background::Task;

type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unknown_worker(id: &str) -> String {
    format!("unknown worker id `{id}` (already merged or discarded?)")
}

/// A tag unique to one [`Delegator`]: the pid and the nanosecond clock tell servers apart
/// (ids are also record file names in a state directory servers share), the instance
/// count tells apart two made in one process at the same instant. Short and filename-safe.
fn run_tag(pid: u32, nanos: u128, instance: u64) -> String {
    format!("{pid:x}_{:x}_{instance:x}", nanos & 0xFF_FFFF_FFFF)
}

pub struct Delegator {
    runtime: Mutex<Runtime>,
    judge: Judge,
    worktrees: Arc<dyn Worktrees>,
    backend_factory: BackendFactory,
    /// The registry. Never held across IO or an await.
    live: Mutex<HashMap<String, Worker>>,
    records: Option<Arc<dyn Records>>,
    /// One record write at a time. Each takes a fresh snapshot of the registry when its turn
    /// comes, so the file always ends up as the registry is, whatever the order of writers.
    gate: Arc<tokio::sync::Mutex<()>>,
    liveness: Arc<dyn Liveness>,
    /// This server, stamped on every record it writes.
    owner: Owner,
    counter: AtomicU64,
    /// Makes worker ids unique across servers that share a state directory.
    run_tag: String,
    now: Clock,
    idle_timeout: Duration,
    log_path: Option<String>,
    planner: String,
    /// The privacy policy (and audit log) briefs answer to, as the planner's own turns do,
    /// and the permission mode a worker's turn runs under.
    policy: Arc<dyn WorkerPolicy>,
    log: Arc<dyn DelegationLog>,
    scan: Scanner,
    /// Runs a finished worker's tests before its work may merge on its own.
    checks: Arc<dyn Checks>,
    /// Work whose tests pass merges without the planner's review (`JEV_AUTO_MERGE=0` turns it off).
    auto_merge: bool,
    /// One change to the main tree at a time: applies and undos share its index and the
    /// snapshot a failed apply restores, so two at once could undo each other's rollback.
    main_tree: tokio::sync::Mutex<()>,
    /// What each auto-merged worker applied, so `undo` can take exactly that back out.
    /// Kept for this server's life only.
    merged: Arc<Mutex<HashMap<String, Applied>>>,
    /// Delegations started in the background and not yet collected by `wait`, by number.
    tasks: Mutex<BTreeMap<u64, Task>>,
    next_task: AtomicU64,
}

/// An auto-merge, as `undo` needs it.
#[derive(Debug, Clone)]
struct Applied {
    patch: String,
    files: Vec<String>,
}

impl Delegator {
    /// The given adapters (clock and idle timeout included); no log, unknown planner, workers not saved.
    pub fn new(runtime: Runtime, judge: Judge, worktrees: Box<dyn Worktrees>, adapters: Adapters) -> Self {
        static INSTANCES: AtomicU64 = AtomicU64::new(0);
        let Adapters { liveness, checks, backend_factory, idle_timeout, log, scan, policy, now } = adapters;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        Self {
            runtime: Mutex::new(runtime),
            judge,
            worktrees: Arc::from(worktrees),
            backend_factory,
            live: Mutex::new(HashMap::new()),
            records: None,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            owner: liveness.me(),
            liveness,
            counter: AtomicU64::new(1),
            run_tag: run_tag(std::process::id(), nanos, INSTANCES.fetch_add(1, Ordering::SeqCst)),
            now,
            idle_timeout,
            log_path: None,
            planner: String::new(),
            policy,
            log,
            scan,
            checks,
            auto_merge: true,
            main_tree: tokio::sync::Mutex::new(()),
            merged: Arc::default(),
            tasks: Mutex::new(BTreeMap::new()),
            next_task: AtomicU64::new(1),
        }
    }

    /// How a finished worker's tests are found and run.
    pub fn with_checks(mut self, checks: Box<dyn Checks>) -> Self {
        self.checks = Arc::from(checks);
        self
    }

    /// `false` sends every editing worker's diff to the planner for review.
    pub fn with_auto_merge(mut self, on: bool) -> Self {
        self.auto_merge = on;
        self
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

    /// Keep the worker registry in these records (the composition root passes the
    /// `<state_dir>/workers` store), so a restarted server still knows its unmerged
    /// workers. Call [`Delegator::recover`] after.
    pub fn with_records(mut self, records: Arc<dyn Records>) -> Self {
        self.records = Some(records);
        self
    }

    /// Who is running, for stamping records and for telling live servers from dead ones.
    #[cfg(test)]
    fn with_liveness(mut self, liveness: Arc<dyn Liveness>) -> Self {
        self.owner = liveness.me();
        self.liveness = liveness;
        self
    }

    fn next_id(&self) -> String {
        format!("w{}-{}", self.run_tag, self.counter.fetch_add(1, Ordering::SeqCst))
    }

    /// The ONE place blocking work leaves the async threads: `f` runs on the blocking pool,
    /// so slow disk and git work of parallel workers overlaps instead of queueing up.
    async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
        tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("blocking task failed: {e}"))
    }

    /// Git work, through [`Worktrees`], via [`Delegator::off_thread`].
    async fn blocking<T: Send + 'static>(&self, f: impl FnOnce(&dyn Worktrees) -> Result<T> + Send + 'static) -> Result<T> {
        let worktrees = self.worktrees.clone();
        Self::off_thread(move || f(&*worktrees)).await?
    }

    // ---- the registry

    /// Write the worker's record as the registry now has it: saved when it has a worktree,
    /// removed when it is gone, nothing while it is only a reserved slot. The registry
    /// lock is held just to copy the entry; the IO runs on the blocking pool, in turn.
    async fn persist(&self, id: &str) {
        let Some(records) = self.records.clone() else { return };
        let _turn = self.gate.lock().await;
        let id = id.to_string();
        let snapshot = lock(&self.live).get(&id).map(|w| Saved::of(&id, w, &self.owner));
        let _ = Self::off_thread(move || match snapshot {
            Some(Some(saved)) => records.save(&saved),
            Some(None) => {}
            None => records.remove(&id),
        })
        .await;
    }

    /// Change a worker's entry and save it. `None` when the id is unknown.
    async fn update<R: Send>(&self, id: &str, f: impl FnOnce(&mut Worker) -> R) -> Option<R> {
        let r = f(lock(&self.live).get_mut(id)?);
        self.persist(id).await;
        Some(r)
    }

    fn lookup(&self, id: &str) -> Result<Worker, String> {
        lock(&self.live).get(id).cloned().ok_or_else(|| unknown_worker(id))
    }

    /// Forget a worker (and its saved record), returning its entry.
    async fn take(&self, id: &str) -> Option<Worker> {
        let w = lock(&self.live).remove(id)?;
        self.persist(id).await;
        Some(w)
    }

    /// Pick a worker for a brief AND claim its slot under one lock, so parallel delegations
    /// see each other and fan out. Lock order is runtime, then the registry.
    fn reserve(&self, signals: &BriefSignals, excluded: &HashSet<(ProviderId, String)>, brief_len: usize, id: &str) -> Result<&'static ModelSpec, NoWorker> {
        let runtime = lock(&self.runtime);
        let mut live = lock(&self.live);
        let spec = {
            let usable = runtime.usable((self.now)()).without(excluded);
            let running = |p: ProviderId| live.values().filter(|w| w.running && w.provider == p).count();
            let utilization = |p: ProviderId| runtime.plan_utilization(p);
            pick_worker(
                signals,
                &WorkerView {
                    usable: &usable,
                    allow: &runtime.allow,
                    overrides: &runtime.overrides,
                    plan_utilization: &utilization,
                    live: &running,
                    context_tokens: (brief_len / 4) as u64,
                },
            )?
        };
        live.insert(id.to_string(), Worker::reserved(spec, self.worktrees.repo(), (self.now)()));
        Ok(spec)
    }

    /// Remove a worker's worktree and forget it. A failed removal is reported but the
    /// worker is forgotten anyway: a worktree that cannot be removed is a leftover to
    /// clean by hand (`git worktree prune`), not a reason to keep tracking it.
    async fn forget(&self, id: &str) -> Result<(), String> {
        let worker = self.take(id).await.ok_or_else(|| unknown_worker(id))?;
        let Some(path) = worker.path else { return Ok(()) };
        let shown = path.clone();
        self.blocking(move |w| w.remove(&path)).await.map_err(|e| format!("worker {id} forgotten, but its worktree {} was not removed: {e}", shown.display()))
    }
}

impl DelegateApi for Arc<Delegator> {
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Ok(Delegator::start(self, req)) })
    }

    fn wait(&self, task_ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::wait(self, task_ids, timeout).await })
    }

    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::show_diff(self, &worker_id, file.as_deref(), offset).await })
    }

    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::merge(self, &worker_id, files).await })
    }

    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::discard(self, &worker_id).await })
    }

    fn undo(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::undo(self, &worker_id).await })
    }
}
