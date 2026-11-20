//! The worker registry on disk: one JSON file per worker under `<state dir>/workers`, what
//! `/workflows` and `/agents` read from it, and the scan startup recovery makes.

use std::path::{Path, PathBuf};

use crate::orchestration::application::ports::{Liveness, Records};
use crate::orchestration::domain::recovery::Found;
use crate::orchestration::domain::worker::Saved;

use super::delegation_log::handoff_key;

/// One JSON file per worker under `<state dir>/workers`, so two servers sharing a state
/// directory never overwrite each other's records. Writes are atomic (temp file, then
/// rename); a missing, unreadable or corrupt file is skipped, never an error.
pub struct Store {
    pub dir: PathBuf,
}

pub(crate) fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Store {
    pub fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn activity_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.activity"))
    }

    /// How many tool calls the worker has made and the latest one, if it has made any.
    pub fn activity(&self, id: &str) -> Option<(usize, String)> {
        let text = std::fs::read_to_string(self.activity_file(id)).ok()?;
        let (count, summary) = text.split_once('\n')?;
        Some((count.parse().ok()?, summary.to_string()))
    }
}

impl Records for Store {
    fn save(&self, s: &Saved) {
        if !plain_id(&s.worker_id) || std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join(format!("{}.json.tmp", s.worker_id));
        let Ok(text) = serde_json::to_string(s) else { return };
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, self.file(&s.worker_id)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn remove(&self, id: &str) {
        if plain_id(id) {
            let _ = std::fs::remove_file(self.file(id));
            let _ = std::fs::remove_file(self.activity_file(id));
        }
    }

    fn note_activity(&self, id: &str, count: usize, summary: &str) {
        if plain_id(id) && self.dir.is_dir() {
            let one_line: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
            let _ = std::fs::write(self.activity_file(id), format!("{count}\n{one_line}"));
        }
    }

    fn load(&self) -> Vec<Saved> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Saved> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| serde_json::from_slice::<Saved>(&std::fs::read(p).ok()?).ok())
            .filter(|s| plain_id(&s.worker_id))
            .collect();
        out.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));
        out
    }
}

/// True when `state` holds worker records or worktrees a previous run left, so a server
/// that never delegates can skip the cleanup.
pub fn has_leftovers(state: &Path) -> bool {
    let any = |d: PathBuf| std::fs::read_dir(d).is_ok_and(|mut e| e.next().is_some());
    any(state.join("workers")) || any(state.join("worktrees"))
}

/// A worker record as `/agents` shows it: plain facts, read from `<state>/workers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWorker {
    pub id: String,
    pub provider: String,
    pub model: String,
    /// Still working (its server has not seen it finish).
    pub running: bool,
    pub created_ms: u64,
    /// Files the planner has already merged back.
    pub merged: Vec<String>,
}

/// The workers recorded under `state`, by id: the unmerged ones a planner can still
/// merge or discard.
pub fn stored_workers(state: &Path) -> Vec<StoredWorker> {
    Store { dir: state.join("workers") }
        .load()
        .into_iter()
        .map(|s| StoredWorker { id: s.worker_id, provider: s.provider, model: s.model, running: s.running, created_ms: s.created_ms, merged: s.merged.into_iter().collect() })
        .collect()
}

/// A worker that is still open (running, or done and waiting for the planner to merge or
/// discard it), as `/workflows` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWorker {
    pub worker_id: String,
    pub provider: String,
    pub model: String,
    pub created_ms: u64,
    pub running: bool,
    /// Files the planner has seen in full, and files already merged.
    pub shown: usize,
    pub merged: usize,
    /// Tool calls made so far, and the latest one (`Bash cargo test`).
    pub tools: usize,
    pub activity: Option<String>,
}

/// The open workers of the project `repo`, from the registry in `<state_dir>/workers`,
/// oldest first. Read only: nothing is adopted, changed or removed.
pub fn open_workers(state_dir: &Path, repo: &Path) -> Vec<OpenWorker> {
    let key = handoff_key(repo);
    let store = Store { dir: state_dir.join("workers") };
    let mut out: Vec<OpenWorker> = store
        .load()
        .into_iter()
        .filter(|s| handoff_key(&s.repo) == key)
        .map(|s| {
            let (tools, activity) = store.activity(&s.worker_id).map_or((0, None), |(n, a)| (n, Some(a)));
            OpenWorker { worker_id: s.worker_id, provider: s.provider, model: s.model, created_ms: s.created_ms, running: s.running, shown: s.shown.len(), merged: s.merged.len(), tools, activity }
        })
        .collect();
    out.sort_by_key(|w| w.created_ms);
    out
}

/// Everything recovery reads from disk, gathered in one blocking step.
pub struct Scan {
    pub records: Vec<Found>,
    /// Directories under the worktrees root, with their modification time.
    pub dirs: Vec<(PathBuf, Option<u64>)>,
}

impl Scan {
    pub fn gather(records: &dyn Records, liveness: &dyn Liveness, root: &Path) -> Self {
        let me = liveness.me();
        let records = records
            .load()
            .into_iter()
            .map(|saved| Found {
                exists: saved.path.exists(),
                owner_alive: saved.owner.as_ref().is_some_and(|o| *o == me || liveness.alive(o)),
                saved,
            })
            .collect();
        let dirs = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .map(|p| {
                let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                let ms = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
                (p, ms)
            })
            .collect();
        Self { records, dirs }
    }
}
