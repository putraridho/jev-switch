//! What a model's own accepted work says about it, and the one place routing lets that
//! reorder the static chains.
//!
//! The chains are the cold-start answer and stay in charge: evidence only reorders models
//! of the same tier that sit next to each other in a chain, never moves a model past a
//! different tier, and only moves models that each have enough verified turns.
//! Hard rules (security, privacy, budget, availability), pins, user chains and quota
//! steering are applied before this and are not seen by it.

use std::collections::HashMap;

use jev_catalog::domain::model_table::{ModelSpec, TaskType};
use jev_shared::ProviderId;

/// Verified turns (accepted or rejected) a model needs, per task type, before its record
/// counts. One or two turns say nothing about a model.
pub const MIN_VERIFIED: u32 = 5;

/// Width of one accepted-rate band, on the lower end of the confidence interval: models in
/// the same band count as equal and the cheaper per accepted task wins.
const BAND: f64 = 0.1;

/// The 95% Wilson score interval of a share of `k` in `n`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub point: f64,
    pub low: f64,
    pub high: f64,
}

impl Rate {
    /// `None` without any trial.
    pub fn of(k: u32, n: u32) -> Option<Self> {
        if n == 0 {
            return None;
        }
        let (k, n) = (f64::from(k), f64::from(n));
        let z = 1.96_f64;
        let p = k / n;
        let denom = 1.0 + z * z / n;
        let centre = (p + z * z / (2.0 * n)) / denom;
        let half = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt() / denom;
        Some(Self { point: p, low: (centre - half).max(0.0), high: (centre + half).min(1.0) })
    }
}

/// How one model did on one kind of task, over the turns whose work was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackRecord {
    pub verified: u32,
    pub accepted: u32,
    /// Tokens spent per accepted turn, when any were accepted and the spend was reported.
    pub tokens_per_accepted: Option<u64>,
}

impl TrackRecord {
    pub fn enough(&self) -> bool {
        self.verified >= MIN_VERIFIED
    }

    fn band(&self) -> i64 {
        let low = Rate::of(self.accepted, self.verified).map_or(0.0, |r| r.low);
        (low / BAND).floor() as i64
    }
}

/// Records by model and task type; empty means "no evidence", which leaves every chain as is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackRecords {
    by: HashMap<(ProviderId, String, TaskType), TrackRecord>,
}

impl TrackRecords {
    pub fn insert(&mut self, provider: ProviderId, id: &str, task: TaskType, record: TrackRecord) {
        self.by.insert((provider, id.to_string(), task), record);
    }

    pub fn get(&self, spec: &ModelSpec, task: TaskType) -> Option<&TrackRecord> {
        self.by.get(&(spec.provider, spec.id.to_string(), task))
    }

    pub fn is_empty(&self) -> bool {
        self.by.is_empty()
    }
}

/// `chain` with each run of equal-tier neighbours ordered by observed record: a higher band
/// of accepted-rate first, then fewer tokens per accepted turn, then the chain's own order.
/// Only models whose record is [`TrackRecord::enough`] trade places, among the places they
/// already hold; security work is never reordered.
pub fn order_by_evidence(chain: &[&'static ModelSpec], task: TaskType, records: &TrackRecords) -> Vec<&'static ModelSpec> {
    let mut out = Vec::with_capacity(chain.len());
    if task == TaskType::Security {
        return chain.to_vec();
    }
    let mut start = 0;
    while start < chain.len() {
        let mut end = start + 1;
        while end < chain.len() && chain[end].tier == chain[start].tier {
            end += 1;
        }
        let mut run = chain[start..end].to_vec();
        // Only models with a record that counts trade places, and only among the places they
        // already hold: a model without evidence keeps its slot.
        let slots: Vec<usize> = (0..run.len()).filter(|i| records.get(run[*i], task).is_some_and(TrackRecord::enough)).collect();
        if slots.len() > 1 {
            let mut known: Vec<(&'static ModelSpec, TrackRecord)> = slots.iter().filter_map(|i| records.get(run[*i], task).map(|r| (run[*i], *r))).collect();
            known.sort_by_key(|(_, r)| (std::cmp::Reverse(r.band()), r.tokens_per_accepted.unwrap_or(u64::MAX)));
            for (slot, (spec, _)) in slots.iter().zip(known) {
                run[*slot] = spec;
            }
        }
        out.extend(run);
        start = end;
    }
    out
}

#[cfg(test)]
mod tests;
