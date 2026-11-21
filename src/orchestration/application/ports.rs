//! The doors the delegator talks through. Real adapters live in
//! `orchestration::infrastructure`; tests use fakes.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::judgment::application::judge::Router;
use crate::judgment::domain::want::ProviderWant;
use crate::orchestration::domain::delegation::DelegateRequest;
use crate::orchestration::domain::worker::{Owner, Saved};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The only door to git. Real: [`GitWorktrees`]; tests use a fake. Every method blocks;
/// the [`Delegator`] runs them off the async threads in one place.
pub trait Worktrees: Send + Sync {
    /// The repository workers start from and merges apply to.
    fn repo(&self) -> PathBuf;
    /// The directory every worktree lives under.
    fn root(&self) -> PathBuf;
    /// A fresh isolated working copy at the main tree's `HEAD`, named `id`.
    fn create(&self, id: &str) -> Result<PathBuf>;
    /// Everything the worker changed in `path` (new files included) as one patch.
    fn diff(&self, path: &Path) -> Result<String>;
    /// Apply `patch` to the MAIN working tree. On failure the tree is left as it was.
    fn apply(&self, patch: &str) -> Result<()>;
    /// Take an applied `patch` back out of the MAIN working tree. On failure nothing changes.
    fn unapply(&self, patch: &str) -> Result<()>;
    /// Delete a worktree checkout.
    fn remove(&self, path: &Path) -> Result<()>;
    /// Forget the bookkeeping of worktrees whose directory is gone (`git worktree prune`).
    fn prune(&self, repo: &Path) -> Result<()>;
}

/// Who is running. Real: [`ProcLiveness`]; tests inject a fake, so no test needs real pids.
pub trait Liveness: Send + Sync {
    /// This process.
    fn me(&self) -> Owner;
    /// Is that process still running? Unsure counts as yes: nothing of a live server may
    /// be touched, and a dead one's records only wait longer.
    fn alive(&self, owner: &Owner) -> bool;
}

/// Where worker records live. Every method blocks on disk: the [`Delegator`] calls them
/// only off the async threads and never under its registry lock. Real: [`Store`].
pub trait Records: Send + Sync {
    fn save(&self, s: &Saved);
    fn remove(&self, id: &str);
    fn load(&self) -> Vec<Saved>;
    /// The worker just began a tool call (`summary`), the `count`th. Kept beside its record
    /// so a screen can show what a running worker is doing; nothing depends on it.
    fn note_activity(&self, _id: &str, _count: usize, _summary: &str) {}
}

/// What the MCP server calls. Implemented by [`Delegator`]; tests use a fake.
pub trait DelegateApi: Send + Sync {
    /// Start a delegation in the background; its task id, or why delegation is unavailable.
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>>;
    /// Collect background tasks (`task_ids`, or all), waiting at most `timeout` when given.
    fn wait(&self, task_ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>>;
    /// One page of a worker's diff, a single file's when `file` is given, from character
    /// `offset`. What it shows in full counts as seen for `merge`.
    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>>;
    /// Apply the worker's diff (only `files` when given) to the main tree, refusing any
    /// file not shown in full; remove its worktree once nothing is left.
    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>>;
    /// Remove the worker's worktree without applying anything.
    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>>;
    /// Take an auto-merged worker's changes back out of the main tree.
    fn undo(&self, worker_id: String) -> BoxFut<'_, Result<String, String>>;
}

/// Jev's judgment of a brief: the same single batched request routing uses.
pub type Judge = Box<dyn Fn(String) -> BoxFut<'static, Option<ProviderWant>> + Send + Sync>;

/// A [`Judge`] that asks `router` (`Router::provider_want`, one HTTP request).
pub fn router_judge(router: Arc<Router>) -> Judge {
    Box::new(move |brief| {
        let router = router.clone();
        // A worker never orchestrates, so the sequence veto is not asked.
        Box::pin(async move { router.provider_want(&brief, "", None, false).await })
    })
}

/// How one test run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckRun {
    Passed,
    /// It exited non-zero or was killed by a signal; the end of its output.
    Failed { output: String },
    /// It ran past the timeout and was killed.
    TimedOut { after: Duration },
    /// The delegation was cancelled while it ran; it was killed.
    Stopped,
    /// It could not be started at all.
    CouldNotStart { error: String },
}

/// Finds and runs a project's tests. Real: [`ProjectChecks`]; tests use a fake.
pub trait Checks: Send + Sync {
    /// The command that tests the project at `dir`, or `None` when there is none.
    fn test_command(&self, dir: &Path) -> Option<String>;
    /// Run `command` in `dir` until it ends, the timeout passes, or `stop()` says to give up.
    /// Blocks: the caller runs it off the async threads.
    fn run(&self, dir: &Path, command: &str, stop: &dyn Fn() -> bool) -> CheckRun;
}
