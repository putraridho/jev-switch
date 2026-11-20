//! A worker in the registry, the server that owns it, and the plain record it is saved as.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::backend::ProviderId;
use crate::catalog::domain::model_table::ModelSpec;

/// A worker started by this server and not yet merged or discarded.
#[derive(Debug, Clone)]
pub struct Worker {
    pub provider: ProviderId,
    pub model: String,
    /// The repository the worker was cut from.
    pub repo: PathBuf,
    pub created_ms: u64,
    /// `None` while the slot is reserved and the worktree is still being made.
    pub path: Option<PathBuf>,
    /// Still running; a finished worker awaiting merge or discard is not load.
    pub running: bool,
    /// Files the planner has seen IN FULL: only these may be merged.
    pub shown: BTreeSet<String>,
    /// Files being read page by page: how many leading characters were seen.
    pub covered: BTreeMap<String, usize>,
    /// Files already merged by an earlier `merge` with `files`.
    pub merged: BTreeSet<String>,
}

impl Worker {
    /// A slot claimed before the worktree exists, so parallel picks see it as load.
    pub fn reserved(spec: &ModelSpec, repo: PathBuf, now_ms: u64) -> Self {
        Self {
            provider: spec.provider,
            model: spec.id.to_string(),
            repo,
            created_ms: now_ms,
            path: None,
            running: true,
            shown: BTreeSet::new(),
            covered: BTreeMap::new(),
            merged: BTreeSet::new(),
        }
    }

    /// Characters `from..to` of `file`'s diff (`total` long) were shown. The file counts as
    /// shown in full once its leading characters were seen without a gap.
    pub fn cover(&mut self, file: &str, from: usize, to: usize, total: usize) {
        if self.shown.contains(file) {
            return;
        }
        let seen = self.covered.get(file).copied().unwrap_or(0);
        if from > seen {
            return;
        }
        if seen.max(to) >= total {
            self.covered.remove(file);
            self.shown.insert(file.to_string());
        } else {
            self.covered.insert(file.to_string(), seen.max(to));
        }
    }
}

/// A server process: its pid plus a start-time token that survives pid reuse (a new
/// process with a recycled pid has a different start time). `start` is `None` where the
/// platform cannot say, and the pid alone is compared then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub pid: u32,
    #[serde(default)]
    pub start: Option<u64>,
}

/// A worker as written to disk. Plain data: provider as its name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    pub worker_id: String,
    pub path: PathBuf,
    pub repo: PathBuf,
    pub provider: String,
    pub model: String,
    pub created_ms: u64,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub shown: BTreeSet<String>,
    #[serde(default)]
    pub covered: BTreeMap<String, usize>,
    #[serde(default)]
    pub merged: BTreeSet<String>,
    /// The server that owns the worker. A record without one (written before owners were
    /// stamped) counts as owned by a dead server.
    #[serde(default)]
    pub owner: Option<Owner>,
}

impl Saved {
    /// `None` for a slot with no worktree yet: there is nothing on disk to keep track of.
    pub fn of(id: &str, w: &Worker, owner: &Owner) -> Option<Self> {
        Some(Self {
            owner: Some(owner.clone()),
            worker_id: id.to_string(),
            path: w.path.clone()?,
            repo: w.repo.clone(),
            provider: w.provider.to_string(),
            model: w.model.clone(),
            created_ms: w.created_ms,
            running: w.running,
            shown: w.shown.clone(),
            covered: w.covered.clone(),
            merged: w.merged.clone(),
        })
    }

    pub fn into_worker(self) -> Option<(String, Worker)> {
        let provider = self.provider.parse().ok()?;
        let w = Worker {
            provider,
            model: self.model,
            repo: self.repo,
            created_ms: self.created_ms,
            path: Some(self.path),
            running: false,
            shown: self.shown,
            covered: self.covered,
            merged: self.merged,
        };
        Some((self.worker_id, w))
    }
}
