//! Delegations that run in the background: `start` returns a task id at once, `wait` collects.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::outcome::Finished;
use super::{lock, Delegator};
use crate::domain::delegation::{DelegateRequest, DelegateResult};

/// A delegation running in the background ([`Delegator::start`]).
pub(super) struct Task {
    pub(super) started_ms: u64,
    /// Turns `true` once `outcome` is set. A sender dropped before that means the task died.
    pub(super) done: tokio::sync::watch::Receiver<bool>,
    pub(super) outcome: Arc<Mutex<Option<Finished>>>,
}

impl Task {
    pub(super) fn ended(&self) -> bool {
        *self.done.borrow() || self.done.has_changed().is_err()
    }

    /// The outcome of an ended task; a task that died without one is a failure.
    pub(super) fn into_finished(self) -> Finished {
        lock(&self.outcome).take().unwrap_or_else(|| Finished::done(DelegateResult::failed("the delegated task stopped unexpectedly".into(), vec![])))
    }
}

/// What `wait` found for one task it was asked about.
pub(super) enum Collected {
    Ended(Task),
    Running { started_ms: u64 },
    /// Another `wait` collected it meanwhile.
    Gone,
}

pub(super) fn task_id(n: u64) -> String {
    format!("t{n}")
}

pub(super) fn task_number(id: &str) -> Option<u64> {
    id.strip_prefix('t')?.parse().ok()
}

impl Delegator {
    /// Start a delegation in the background and return its task id at once, so the planner
    /// keeps working meanwhile. The worker runs and is tested; nothing lands in the main tree
    /// until [`Delegator::wait`] collects it.
    pub fn start(self: &Arc<Self>, req: DelegateRequest) -> String {
        let n = self.next_task.fetch_add(1, Ordering::SeqCst);
        let (tx, done) = tokio::sync::watch::channel(false);
        let outcome = Arc::new(Mutex::new(None));
        lock(&self.tasks).insert(n, Task { started_ms: (self.now)(), done, outcome: outcome.clone() });
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let finished = me.run(req).await;
            *lock(&outcome) = Some(finished);
            let _ = tx.send(true);
        });
        task_id(n)
    }

    /// Wait for background tasks (`ids`, or every one not yet collected) to end, at most
    /// `timeout` when given, then land and report each that ended. Each task is collected
    /// once; the ones still running are listed.
    pub async fn wait(&self, ids: Option<Vec<String>>, timeout: Option<Duration>) -> Result<String, String> {
        let wanted = self.wanted_tasks(ids)?;
        if wanted.is_empty() {
            return Ok("no delegated tasks to wait for: every task has been collected".into());
        }
        let receivers: Vec<_> = {
            let tasks = lock(&self.tasks);
            wanted.iter().filter_map(|n| tasks.get(n)).map(|t| t.done.clone()).collect()
        };
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
        for mut rx in receivers {
            // An error means the task died: it counts as ended and is reported below.
            let ended = async move {
                let _ = rx.wait_for(|done| *done).await;
            };
            let in_time = match deadline {
                Some(at) => tokio::time::timeout_at(at, ended).await.is_ok(),
                None => {
                    ended.await;
                    true
                }
            };
            if !in_time {
                break;
            }
        }
        let mut blocks: Vec<String> = Vec::new();
        let mut running: Vec<String> = Vec::new();
        for n in wanted {
            let collected = {
                let mut tasks = lock(&self.tasks);
                match tasks.get(&n) {
                    None => Collected::Gone,
                    Some(t) if t.ended() => tasks.remove(&n).map_or(Collected::Gone, Collected::Ended),
                    Some(t) => Collected::Running { started_ms: t.started_ms },
                }
            };
            match collected {
                Collected::Ended(task) => {
                    let result = self.settle(task.into_finished()).await;
                    blocks.push(format!("task {}: {}", task_id(n), result.render()));
                }
                Collected::Running { started_ms } => {
                    let took = jev_shared::domain::wording::human_duration((self.now)().saturating_sub(started_ms));
                    running.push(format!("{} (running for {took})", task_id(n)));
                }
                Collected::Gone => blocks.push(format!("task {}: already collected by another wait", task_id(n))),
            }
        }
        if !running.is_empty() {
            let waited = timeout.map_or(String::new(), |t| format!(" after {}", jev_shared::domain::wording::human_duration(t.as_millis() as u64)));
            blocks.push(format!("Still running{waited}: {}. Call wait again to collect them.", running.join(", ")));
        }
        Ok(blocks.join("\n\n"))
    }

    /// The task numbers `ids` names, each once, or every uncollected task when `None`.
    pub(super) fn wanted_tasks(&self, ids: Option<Vec<String>>) -> Result<Vec<u64>, String> {
        let tasks = lock(&self.tasks);
        let Some(ids) = ids else { return Ok(tasks.keys().copied().collect()) };
        if ids.is_empty() {
            return Err("`task_ids` is empty: name the tasks, or leave it out to wait for every task".into());
        }
        let mut wanted: Vec<u64> = Vec::new();
        for id in &ids {
            let n = task_number(id).filter(|n| tasks.contains_key(n)).ok_or_else(|| {
                let open: Vec<String> = tasks.keys().map(|n| task_id(*n)).collect();
                let open = if open.is_empty() { "none".to_string() } else { open.join(", ") };
                format!("unknown task `{id}` (already collected?); uncollected tasks: {open}")
            })?;
            if !wanted.contains(&n) {
                wanted.push(n);
            }
        }
        Ok(wanted)
    }
}
