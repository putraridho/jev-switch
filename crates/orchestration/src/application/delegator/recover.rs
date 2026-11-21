//! Startup recovery: reload what an earlier run left and clean up the rest.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use crate::domain::recovery::{Fate, ORPHAN_GRACE_MS};

use super::{lock, Delegator};

impl Delegator {
    /// At startup: reload the workers an earlier run left, and clean up. Records of a
    /// server that is still running are NEVER touched, adopted or removed: they are in use.
    /// Of a dead server's, finished workers are reloaded (stamped as this server's now),
    /// and those older than 24 hours lose their checkout and record. Directories under the
    /// worktrees root that no record names (and that are not brand new) go too; `git
    /// worktree prune` runs in every repository touched. All disk work is off the async threads.
    pub async fn recover(&self) {
        let Some(records) = self.records.clone() else { return };
        let (liveness, root) = (self.liveness.clone(), self.worktrees.root());
        let scan = self.scan.clone();
        let Ok(scan) = Self::off_thread(move || scan(&*records, &*liveness, &root)).await else { return };
        let now = (self.now)();
        let known: HashSet<&PathBuf> = scan.records.iter().map(|f| &f.saved.path).collect();
        let mut repos: BTreeSet<PathBuf> = BTreeSet::new();
        for found in &scan.records {
            match found.fate(now) {
                Fate::Keep => {}
                Fate::Drop => {
                    self.drop_checkout(found.saved.path.clone()).await;
                    self.persist(&found.saved.worker_id).await;
                    repos.insert(found.saved.repo.clone());
                }
                Fate::Adopt => {
                    if let Some((id, w)) = found.saved.clone().into_worker() {
                        lock(&self.live).insert(id.clone(), w);
                        self.persist(&id).await;
                    }
                }
            }
        }
        for (path, modified_ms) in scan.dirs.iter().filter(|(p, _)| !known.contains(p)) {
            if now.saturating_sub(modified_ms.unwrap_or(now)) > ORPHAN_GRACE_MS {
                self.drop_checkout(path.clone()).await;
                repos.insert(self.worktrees.repo());
            }
        }
        for repo in repos {
            let _ = self.blocking(move |w| w.prune(&repo)).await;
        }
    }

    pub(super) async fn drop_checkout(&self, path: PathBuf) {
        let _ = self.blocking(move |w| w.remove(&path)).await;
    }
}
