//! What startup recovery decides about a saved worker record, from facts the caller
//! gathered off the disk and the process table.

use super::worker::Saved;

/// A worker whose record is older than this is abandoned: its checkout is removed.
pub const STALE_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
/// A directory under the worktrees root that no record names is removed once it is this
/// old; younger ones may belong to a server that is creating them right now.
pub const ORPHAN_GRACE_MS: u64 = 10 * 60 * 1000;

/// A saved record and what recovery learned about it from the disk and the process table.
pub struct Found {
    pub saved: Saved,
    /// The checkout directory is still there.
    pub exists: bool,
    /// The owner is this process or another one still running.
    pub owner_alive: bool,
}

/// What recovery does with a record.
#[derive(Debug, PartialEq, Eq)]
pub enum Fate {
    /// Reload it, so merge and discard still work after a restart.
    Adopt,
    /// Remove its checkout and record.
    Drop,
    /// Leave it alone.
    Keep,
}

impl Found {
    /// A record whose owner is alive is never touched, however old: it is in use. Of a
    /// dead owner's: a finished worker is adopted; one that was still running when its
    /// server died is not reloaded (no planner was ever told its id) and waits to age out;
    /// and old or checkout-less ones are dropped.
    pub fn fate(&self, now_ms: u64) -> Fate {
        let s = &self.saved;
        if self.owner_alive {
            Fate::Keep
        } else if now_ms.saturating_sub(s.created_ms) > STALE_AFTER_MS || (!s.running && !self.exists) {
            Fate::Drop
        } else if s.running {
            Fate::Keep
        } else {
            Fate::Adopt
        }
    }
}

/// Everything recovery reads from disk, gathered in one blocking step.
pub struct Scan {
    pub records: Vec<Found>,
    /// Directories under the worktrees root, with their modification time.
    pub dirs: Vec<(std::path::PathBuf, Option<u64>)>,
}
