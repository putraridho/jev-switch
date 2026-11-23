//! The usage ledger: what every process of a session has spent and what is reserved in
//! flight, as one value ([`Book`]) that the processes change one at a time.
//!
//! A planner's own turns and the workers its delegate tool starts (a separate process) all
//! admit an attempt against it ([`super::admission::admit`]), which reserves the attempt's
//! estimated maximum ([`Held`]); the finished attempt settles the reservation with what it
//! really used ([`Book::settle`]). A reservation whose owner died is reaped
//! ([`Book::reap`]) and counted as spend nobody reported, never as zero.
//!
//! Pure: time and "is that process alive" come in as arguments.

use std::collections::BTreeMap;

use jev_backend::{ProviderId, Usage};
use jev_providers::domain::funding::Fundings;
use jev_shared::Measurement;

use super::budget::{Meter, PriceTable};

/// A reservation older than this is reaped even when its owner looks alive (a recycled pid).
pub const MAX_HOLD_AGE_MS: u64 = 4 * 60 * 60 * 1000;
/// A session nobody touched for this long is not the one a worker joins.
pub const SESSION_IDLE_MS: u64 = 6 * 60 * 60 * 1000;
/// Sessions older than this are dropped from the file.
const RETAIN_MS: u64 = 8 * 24 * 60 * 60 * 1000;
const MAX_SESSIONS: usize = 40;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// The writer name of spend recorded for a reservation that was reaped.
pub const REAPED_WRITER: &str = "reaped";

/// An attempt's reserved maximum, held until the attempt settles or its owner dies.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub id: String,
    pub owner_pid: u32,
    /// The owner's start time when the platform can say, against a recycled pid.
    pub owner_start: Option<u64>,
    pub ts_ms: u64,
    pub provider: ProviderId,
    pub model: String,
    /// Whether the attempt costs money (see [`Fundings::is_paid`]); decided when it was admitted.
    pub paid: bool,
    pub tokens: u64,
    /// The reserved dollars; `None` when the model has no price.
    pub usd: Option<f64>,
}

/// One finished attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct Spent {
    pub ts_ms: u64,
    /// The process that ran it.
    pub writer: String,
    pub provider: ProviderId,
    pub model: String,
    pub usage: Usage,
}

impl Spent {
    /// An attempt whose consumption nobody knows.
    pub fn unknown(ts_ms: u64, writer: &str, provider: ProviderId, model: &str) -> Self {
        Self { ts_ms, writer: writer.to_string(), provider, model: model.to_string(), usage: Usage { measurement: Measurement::Unavailable, ..Usage::default() } }
    }
}

/// What is reserved in flight, by what it would cost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reserved {
    /// Attempts in flight, paid or not.
    pub attempts: usize,
    /// Reserved by paid attempts.
    pub paid_tokens: u64,
    pub paid_usd: f64,
    /// Paid reservations of a model with no price, so `paid_usd` leaves them out.
    pub unpriced: u32,
    /// Reserved by plan attempts, by provider.
    pub plan_tokens: BTreeMap<ProviderId, u64>,
}

impl Reserved {
    pub fn add(&mut self, held: &Held) {
        self.attempts += 1;
        if held.paid {
            self.paid_tokens += held.tokens;
            match held.usd {
                Some(usd) => self.paid_usd += usd,
                None => self.unpriced += 1,
            }
        } else {
            *self.plan_tokens.entry(held.provider).or_default() += held.tokens;
        }
    }

    pub fn absorb(&mut self, other: &Reserved) {
        self.attempts += other.attempts;
        self.paid_tokens += other.paid_tokens;
        self.paid_usd += other.paid_usd;
        self.unpriced += other.unpriced;
        for (p, t) in &other.plan_tokens {
            *self.plan_tokens.entry(*p).or_default() += t;
        }
    }
}

/// One session of one project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub project: String,
    pub started_ms: u64,
    pub held: Vec<Held>,
    pub spent: Vec<Spent>,
}

impl Session {
    /// The spend as a meter (so unreported and unpriced attempts count the way they do for a
    /// process's own spend), leaving out the lines `skip_writer` wrote.
    pub fn meter(&self, prices: &PriceTable, fundings: &Fundings, skip_writer: Option<&str>) -> Meter {
        meter_of(self.spent.iter(), prices, fundings, skip_writer)
    }

    pub fn reserved(&self) -> Reserved {
        let mut reserved = Reserved::default();
        self.held.iter().for_each(|h| reserved.add(h));
        reserved
    }

    fn last_activity_ms(&self) -> u64 {
        let spent = self.spent.iter().map(|s| s.ts_ms);
        spent.chain(self.held.iter().map(|h| h.ts_ms)).fold(self.started_ms, u64::max)
    }
}

fn meter_of<'a>(spent: impl Iterator<Item = &'a Spent>, prices: &PriceTable, fundings: &Fundings, skip_writer: Option<&str>) -> Meter {
    let mut meter = Meter::default();
    meter.set_fundings(fundings.clone());
    for s in spent.filter(|s| skip_writer != Some(s.writer.as_str())) {
        meter.record(s.provider, &s.model, &s.usage, prices);
    }
    meter
}

/// Every session in the ledger file, and which one each project is in now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book {
    /// Project folder to the id of the session a worker of it joins.
    pub current: BTreeMap<String, String>,
    pub sessions: BTreeMap<String, Session>,
}

impl Book {
    /// A planner starts session `id` of `project`; workers of the project join it.
    pub fn begin(&mut self, id: &str, project: &str, now_ms: u64) {
        self.sessions.entry(id.to_string()).or_insert_with(|| Session { project: project.to_string(), started_ms: now_ms, ..Session::default() });
        self.current.insert(project.to_string(), id.to_string());
        self.prune(now_ms);
    }

    /// The session a process with no planner of its own (a worker) joins: its project's current
    /// one unless nobody touched it for [`SESSION_IDLE_MS`], in which case `fresh_id` starts one.
    pub fn join(&mut self, project: &str, now_ms: u64, fresh_id: impl FnOnce() -> String) -> String {
        let live = self.current.get(project).filter(|id| self.sessions.get(*id).is_some_and(|s| now_ms.saturating_sub(s.last_activity_ms()) < SESSION_IDLE_MS)).cloned();
        live.unwrap_or_else(|| {
            let id = fresh_id();
            self.begin(&id, project, now_ms);
            id
        })
    }

    fn prune(&mut self, now_ms: u64) {
        let current: Vec<String> = self.current.values().cloned().collect();
        self.sessions.retain(|id, s| current.contains(id) || !s.held.is_empty() || now_ms.saturating_sub(s.last_activity_ms()) < RETAIN_MS);
        while self.sessions.len() > MAX_SESSIONS {
            let oldest = self.sessions.iter().filter(|(id, s)| !current.contains(id) && s.held.is_empty()).min_by_key(|(_, s)| s.last_activity_ms()).map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.sessions.remove(&id),
                None => break,
            };
        }
        self.current.retain(|_, id| self.sessions.contains_key(id));
    }

    /// Drop reservations whose owner is gone (`alive` says so) or that are older than
    /// [`MAX_HOLD_AGE_MS`]. What such an attempt used is unknown, so each becomes unreported
    /// spend. Returns how many were reaped.
    pub fn reap(&mut self, now_ms: u64, alive: &dyn Fn(&Held) -> bool) -> usize {
        let mut reaped = 0;
        for session in self.sessions.values_mut() {
            let (stale, kept): (Vec<Held>, Vec<Held>) = std::mem::take(&mut session.held).into_iter().partition(|h| now_ms.saturating_sub(h.ts_ms) > MAX_HOLD_AGE_MS || !alive(h));
            session.held = kept;
            reaped += stale.len();
            session.spent.extend(stale.iter().map(|h| Spent::unknown(now_ms, REAPED_WRITER, h.provider, &h.model)));
        }
        reaped
    }

    pub fn hold(&mut self, session: &str, held: Held) {
        if let Some(s) = self.sessions.get_mut(session) {
            s.held.push(held);
        }
    }

    /// End a reservation with what the attempt used. The spend goes into the session the
    /// reservation was in, or `session` when it was reaped meanwhile.
    pub fn settle(&mut self, session: &str, hold_id: Option<&str>, spent: Spent) {
        let home = hold_id.and_then(|id| self.release(id)).unwrap_or_else(|| session.to_string());
        if let Some(s) = self.sessions.get_mut(&home) {
            s.spent.push(spent);
        }
    }

    /// Remove a reservation without spend; the id of its session when there was one.
    pub fn release(&mut self, hold_id: &str) -> Option<String> {
        self.sessions.iter_mut().find_map(|(id, s)| {
            let at = s.held.iter().position(|h| h.id == hold_id)?;
            s.held.remove(at);
            Some(id.clone())
        })
    }

    /// Everything reserved in flight, over every session.
    pub fn reserved_all(&self) -> Reserved {
        let mut all = Reserved::default();
        self.sessions.values().for_each(|s| all.absorb(&s.reserved()));
        all
    }

    /// The spend of every session on the UTC day that `now_ms` is in.
    pub fn day_meter(&self, now_ms: u64, prices: &PriceTable, fundings: &Fundings) -> Meter {
        let start = now_ms - now_ms % DAY_MS;
        meter_of(self.sessions.values().flat_map(|s| s.spent.iter()).filter(|s| s.ts_ms >= start), prices, fundings, None)
    }
}

#[cfg(test)]
mod tests;
