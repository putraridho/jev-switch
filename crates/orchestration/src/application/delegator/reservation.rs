//! A delegation's claim on its worker slot while it runs, released if the call is dropped.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

use crate::application::ports::Worktrees;

use super::{lock, Delegator};

/// Where a worktree being created stands, shared between the blocking task making it and
/// the [`Reservation`] that may be dropped meanwhile.
#[derive(Default)]
pub(super) struct CreateSlot {
    /// The delegation was dropped: whatever gets made must be removed again.
    pub(super) cancelled: bool,
    /// The worktree, once made and not cancelled.
    pub(super) path: Option<PathBuf>,
}

/// `create`, then record the path in `slot`, all-or-nothing against a cancel: whichever of
/// this and [`Reservation`]'s drop takes the slot lock second sees the other's work and
/// removes the worktree, so none is ever left behind.
pub(super) fn create_unless_cancelled(w: &dyn Worktrees, id: &str, slot: &Mutex<CreateSlot>) -> Result<PathBuf> {
    let path = w.create(id)?;
    let mut s = lock(slot);
    if s.cancelled {
        drop(s);
        let _ = w.remove(&path);
        bail!("the delegation was cancelled");
    }
    s.path = Some(path.clone());
    Ok(path)
}

/// Owns a worker's reserved slot while its delegation runs. Dropped before [`disarm`] (the
/// `delegate` future was dropped, so nobody will ever merge or discard this worker) it
/// removes the placeholder from the registry, its record and its worktree, so a cancelled
/// call is not counted as load forever.
///
/// [`disarm`]: Reservation::disarm
pub(super) struct Reservation<'a> {
    pub(super) delegator: &'a Delegator,
    pub(super) id: String,
    pub(super) slot: Arc<Mutex<CreateSlot>>,
    pub(super) armed: bool,
}

impl<'a> Reservation<'a> {
    pub(super) fn new(delegator: &'a Delegator, id: &str) -> Self {
        Self { delegator, id: id.to_string(), slot: Arc::default(), armed: true }
    }

    pub(super) fn slot(&self) -> Arc<Mutex<CreateSlot>> {
        self.slot.clone()
    }

    /// The delegation ended on its own: the registry already says what is left.
    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let d = self.delegator;
        lock(&d.live).remove(&self.id);
        let path = {
            let mut s = lock(&self.slot);
            s.cancelled = true;
            s.path.take()
        };
        // Drop cannot await, and the disk work must not run on whichever thread dropped
        // the future: a plain thread, which takes its turn among the record writers.
        let (worktrees, records, gate, id) = (d.worktrees.clone(), d.records.clone(), d.gate.clone(), self.id.clone());
        std::thread::spawn(move || {
            let _turn = gate.blocking_lock();
            if let Some(path) = path {
                let _ = worktrees.remove(&path);
            }
            if let Some(records) = records {
                records.remove(&id);
            }
        });
    }
}
