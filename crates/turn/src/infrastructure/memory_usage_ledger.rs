//! The usage ledger in memory: for tests, and for a process with no state folder. Every handle
//! to the same `Arc` shares it, as handles to one file do; the path is ignored.

use std::path::Path;
use std::sync::Mutex;

use crate::application::ports::UsageLedger;
use crate::domain::usage_ledger::Book;

#[derive(Debug, Default)]
pub struct MemoryUsageLedger(Mutex<Book>);

impl MemoryUsageLedger {
    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl UsageLedger for MemoryUsageLedger {
    fn transact(&self, _path: &Path, change: &mut dyn FnMut(&mut Book)) -> bool {
        change(&mut self.book());
        true
    }

    fn read(&self, _path: &Path) -> Option<Book> {
        Some(self.book().clone())
    }
}

/// Which processes are running, for tests: every pid is alive unless it was told otherwise.
#[derive(Debug, Default)]
pub struct KnownProcesses {
    me: u32,
    dead: Mutex<Vec<u32>>,
}

impl KnownProcesses {
    pub fn new(me: u32) -> Self {
        Self { me, dead: Mutex::default() }
    }

    /// `pid` is no longer running.
    pub fn die(&self, pid: u32) {
        self.dead.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
    }
}

impl jev_orchestration::application::ports::Liveness for KnownProcesses {
    fn me(&self) -> jev_orchestration::domain::worker::Owner {
        jev_orchestration::domain::worker::Owner { pid: self.me, start: None }
    }

    fn alive(&self, owner: &jev_orchestration::domain::worker::Owner) -> bool {
        !self.dead.lock().unwrap_or_else(|e| e.into_inner()).contains(&owner.pid)
    }
}
