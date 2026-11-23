//! One process's door to the shared usage ledger: it admits an attempt (reserving its
//! estimated maximum under the ledger's lock), settles it with what it used, and reads the
//! session's and the day's spend back.
//!
//! A planner starts a session of its own; a worker's process (`delegate_mcp`) joins the
//! session its project's planner started.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use jev_backend::{ProviderId, Usage};
use jev_orchestration::application::ports::{BudgetHold, Liveness};
use jev_orchestration::domain::worker::Owner;
use jev_providers::domain::funding::Fundings;

use crate::application::ports::UsageLedger;
use crate::domain::admission::{self, Admission, Attempt, Estimate, Standing};
use crate::domain::budget::{Budget, Meter, PriceTable};
use crate::domain::usage_ledger::{Book, Held, Reserved, Spent};

/// Whose session this process records into.
#[derive(Debug, Clone)]
enum Role {
    /// Started its own session.
    Planner { session: String },
    /// Joins the newest session of its project.
    Worker,
}

pub struct UsageAccount {
    ledger: Arc<dyn UsageLedger>,
    liveness: Arc<dyn Liveness>,
    path: PathBuf,
    project: String,
    role: Role,
    owner: Owner,
    /// One per process, so a process's own lines can be told from the others'.
    writer: String,
    counter: AtomicU64,
    clock: Clock,
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl std::fmt::Debug for UsageAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageAccount").field("path", &self.path).field("project", &self.project).field("role", &self.role).field("writer", &self.writer).finish()
    }
}

/// What the ledger says for `/budget`.
#[derive(Debug, Clone, Default)]
pub struct Overview {
    /// The session's spend, every process's.
    pub session: Meter,
    pub session_reserved: Reserved,
    /// The day's spend over every session.
    pub day: Meter,
}

impl UsageAccount {
    /// A planner: starts a fresh session of `project`, which its workers will join.
    pub fn planner(ledger: Arc<dyn UsageLedger>, liveness: Arc<dyn Liveness>, path: PathBuf, project: &Path, now_ms: u64) -> Self {
        let owner = liveness.me();
        let session = format!("{}-{now_ms}", owner.pid);
        let account = Self::new(ledger, liveness, path, project, Role::Planner { session: session.clone() }, owner, now_ms);
        let project = account.project.clone();
        account.ledger.transact(&account.path, &mut |book| book.begin(&session, &project, now_ms));
        account
    }

    /// A worker's process: joins the session its project's planner started.
    pub fn worker(ledger: Arc<dyn UsageLedger>, liveness: Arc<dyn Liveness>, path: PathBuf, project: &Path, now_ms: u64) -> Self {
        let owner = liveness.me();
        Self::new(ledger, liveness, path, project, Role::Worker, owner, now_ms)
    }

    fn new(ledger: Arc<dyn UsageLedger>, liveness: Arc<dyn Liveness>, path: PathBuf, project: &Path, role: Role, owner: Owner, now_ms: u64) -> Self {
        static HANDLES: AtomicU64 = AtomicU64::new(0);
        let writer = format!("{}-{now_ms}-{}", owner.pid, HANDLES.fetch_add(1, Ordering::Relaxed));
        Self { ledger, liveness, path, project: project.to_string_lossy().into_owned(), role, owner, writer, counter: AtomicU64::new(0), clock: Arc::new(system_clock) }
    }

    /// Read time from `clock` (Unix ms) instead of the system's, for tests.
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn session_in(&self, book: &mut Book, now_ms: u64) -> String {
        match &self.role {
            Role::Planner { session } => {
                if !book.sessions.contains_key(session) {
                    book.begin(session, &self.project, now_ms);
                }
                session.clone()
            }
            Role::Worker => book.join(&self.project, now_ms, || format!("{}-{now_ms}", self.owner.pid)),
        }
    }

    /// The session a read sees, without starting one.
    fn session_of<'a>(&'a self, book: &'a Book) -> Option<&'a str> {
        match &self.role {
            Role::Planner { session } => Some(session),
            Role::Worker => book.current.get(&self.project).map(String::as_str),
        }
    }

    fn alive(&self, held: &Held) -> bool {
        held.owner_pid == self.owner.pid || self.liveness.alive(&Owner { pid: held.owner_pid, start: held.owner_start })
    }

    /// Admit an attempt of `model` on `provider`, reserving `estimate` for it. `Err` is why it
    /// may not start. A ledger that cannot be used refuses a paid attempt (the cap cannot be
    /// kept) and lets a plan attempt through unreserved.
    #[allow(clippy::too_many_arguments)]
    pub fn hold(&self, budget: &Budget, prices: &PriceTable, fundings: &Fundings, provider: ProviderId, model: &str, estimate: Estimate) -> Result<BudgetHold, String> {
        let now_ms = (self.clock)();
        let paid = fundings.is_paid(provider, model);
        let id = format!("{}-{}", self.writer, self.counter.fetch_add(1, Ordering::Relaxed));
        let mut answer: Option<Admission> = None;
        let reached = self.ledger.transact(&self.path, &mut |book| {
            book.reap(now_ms, &|h| self.alive(h));
            let session = self.session_in(book, now_ms);
            let own = book.sessions.get(&session).cloned().unwrap_or_default();
            let (session_meter, day_meter) = (own.meter(prices, fundings, None), book.day_meter(now_ms, prices, fundings));
            let session_reserved = own.reserved();
            let day_reserved = book.reserved_all();
            let verdict = admission::admit(
                &Attempt { provider, paid, estimate, budget },
                &Standing { session: &session_meter, session_reserved: &session_reserved, day: &day_meter, day_reserved: &day_reserved },
            );
            if matches!(verdict, Admission::Admitted { .. }) {
                book.hold(&session, Held { id: id.clone(), owner_pid: self.owner.pid, owner_start: self.owner.start, ts_ms: now_ms, provider, model: model.to_string(), paid, tokens: estimate.tokens, usd: estimate.usd });
            }
            answer = Some(verdict);
        });
        match (reached, answer) {
            (true, Some(Admission::Admitted { warning })) => Ok(BudgetHold { id: Some(id), warning }),
            (true, Some(Admission::Refused(why))) => Err(why),
            _ if paid => Err("the usage ledger could not be used, so the paid cap cannot be kept".into()),
            _ => Ok(BudgetHold::default()),
        }
    }

    /// End a reservation (or record a finished attempt that had none) with what it used.
    pub fn settle(&self, hold: &BudgetHold, provider: ProviderId, model: &str, usage: &Usage) {
        let now_ms = (self.clock)();
        let spent = Spent { ts_ms: now_ms, writer: self.writer.clone(), provider, model: model.to_string(), usage: *usage };
        self.ledger.transact(&self.path, &mut |book| {
            let session = self.session_in(book, now_ms);
            book.settle(&session, hold.id.as_deref(), spent.clone());
        });
    }

    /// End the reservation of an attempt that never started.
    pub fn release(&self, hold: &BudgetHold) {
        if let Some(id) = &hold.id {
            self.ledger.transact(&self.path, &mut |book| {
                book.release(id);
            });
        }
    }

    /// What the other processes of the session spent, as a meter.
    pub fn others(&self, prices: &PriceTable, fundings: &Fundings) -> Meter {
        let book = self.ledger.read(&self.path).unwrap_or_default();
        match self.session_of(&book).and_then(|id| book.sessions.get(id)) {
            Some(session) => session.meter(prices, fundings, Some(&self.writer)),
            None => Meter::default(),
        }
    }

    /// The spend of the day, over every session.
    pub fn day(&self, prices: &PriceTable, fundings: &Fundings) -> Meter {
        self.ledger.read(&self.path).unwrap_or_default().day_meter((self.clock)(), prices, fundings)
    }

    pub fn overview(&self, prices: &PriceTable, fundings: &Fundings) -> Overview {
        let book = self.ledger.read(&self.path).unwrap_or_default();
        let session = self.session_of(&book).and_then(|id| book.sessions.get(id));
        Overview {
            session: session.map_or_else(Meter::default, |s| s.meter(prices, fundings, None)),
            session_reserved: session.map(|s| s.reserved()).unwrap_or_default(),
            day: book.day_meter((self.clock)(), prices, fundings),
        }
    }
}
