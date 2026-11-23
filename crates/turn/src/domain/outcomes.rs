//! Accepted outcomes per (provider, model, task type): how often a model's work held up, how
//! often it had to be redone and what it cost per accepted turn. Built from the audit log
//! and kept current in memory as turns finish. It only counts what was observed: a turn
//! nobody checked is "unverified" and sits in no rate.

use std::collections::{BTreeMap, HashMap};

use jev_catalog::domain::model_table::TaskType;
use jev_routing::domain::track_record::{Rate, TrackRecord, TrackRecords, MIN_VERIFIED};
use jev_shared::domain::acceptance::{Acceptance, Quality, Rejection, Rework};
use jev_shared::domain::audit::{Entry, Event, Outcome, Verdict};
use jev_shared::{Measurement, ProviderId, Usage};

/// The label for a turn whose task type was not recorded.
const UNKNOWN_TASK: &str = "unknown";

/// Tokens a model consumed (the cached share of the input left out); 0 when the CLI did not
/// report them, so unknown spend never reads as a cheap turn in a ranking.
pub fn spent_tokens(usage: &Usage) -> u64 {
    match usage.measurement {
        Measurement::Reported => usage.input_tokens.saturating_sub(usage.cached_input_tokens).saturating_add(usage.output_tokens),
        Measurement::Unavailable => 0,
    }
}

/// What happened to one model's turns of one task type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    pub turns: u32,
    pub accepted: u32,
    pub rejected: u32,
    pub unverified: u32,
    /// Turns that redid the files of a turn that failed its check or was undone.
    pub rework: u32,
    /// Models started beyond the first, summed.
    pub retries: u32,
    /// Tokens of the turns that were checked (accepted or rejected): failures and rework
    /// count against the cost of what was finally accepted.
    pub checked_tokens: u64,
}

impl Tally {
    /// Turns whose work was checked or taken back.
    pub fn verified(&self) -> u32 {
        self.accepted + self.rejected
    }

    /// Accepted share of the verified turns, with its uncertainty; `None` before any.
    pub fn accepted_rate(&self) -> Option<Rate> {
        Rate::of(self.accepted, self.verified())
    }

    /// Whether the verified turns are enough to rank on.
    pub fn enough(&self) -> bool {
        self.verified() >= MIN_VERIFIED
    }

    /// Tokens spent on checked turns per accepted turn; `None` while nothing was accepted or
    /// no tokens were reported.
    pub fn tokens_per_accepted(&self) -> Option<u64> {
        (self.accepted > 0 && self.checked_tokens > 0).then(|| self.checked_tokens / u64::from(self.accepted))
    }

    fn add(&mut self, q: &Quality, acceptance: Acceptance) {
        self.turns += 1;
        self.retries += q.retries;
        self.rework += u32::from(q.rework != Rework::None);
        match acceptance {
            Acceptance::Accepted => self.accepted += 1,
            Acceptance::Rejected(_) => self.rejected += 1,
            Acceptance::Unverified => self.unverified += 1,
        }
        if !matches!(acceptance, Acceptance::Unverified) {
            self.checked_tokens += q.spent_tokens;
        }
    }

    fn merge(&mut self, other: &Tally) {
        self.turns += other.turns;
        self.accepted += other.accepted;
        self.rejected += other.rejected;
        self.unverified += other.unverified;
        self.rework += other.rework;
        self.retries += other.retries;
        self.checked_tokens += other.checked_tokens;
    }
}

/// The turn just before, per project, so an undo found later can take its acceptance back.
#[derive(Debug, Clone)]
struct Last {
    key: (String, String),
    acceptance: Acceptance,
    spent_tokens: u64,
}

/// Every model's tallies by task type.
#[derive(Debug, Clone, Default)]
pub struct Outcomes {
    by: BTreeMap<(String, String), Tally>,
    last: HashMap<String, Last>,
}

impl Outcomes {
    /// The outcomes of every turn in `entries`, oldest first.
    pub fn from_entries(entries: &[Entry]) -> Self {
        let mut out = Self::default();
        for e in entries {
            let Event::Turn { model, outcome, quality, verdict, .. } = &e.event else { continue };
            // A line from before acceptance was recorded still has the check's verdict.
            let mut quality = quality.clone();
            if quality.acceptance == Acceptance::Unverified {
                quality.acceptance = match verdict {
                    Some(Verdict::Passed) => Acceptance::Accepted,
                    Some(Verdict::Failed) => Acceptance::Rejected(Rejection::VerificationFailed),
                    None => Acceptance::Unverified,
                };
            }
            out.add_turn(&e.project, model, *outcome == Outcome::Answered, &quality);
        }
        out
    }

    /// Count a finished turn of `model` (`provider · id`). A turn that did not answer is
    /// never accepted. A turn that redid work the person had undone takes the acceptance of
    /// the turn before it back.
    pub fn add_turn(&mut self, project: &str, model: &str, answered: bool, q: &Quality) {
        if model.is_empty() {
            return;
        }
        let task = if q.task.is_empty() { UNKNOWN_TASK } else { q.task.as_str() };
        let key = (model.to_string(), task.to_string());
        if q.rework == Rework::AfterRevert {
            self.reject_last(project);
        }
        let acceptance = if answered { q.acceptance } else { Acceptance::Unverified };
        self.by.entry(key.clone()).or_default().add(q, acceptance);
        self.last.insert(project.to_string(), Last { key, acceptance, spent_tokens: q.spent_tokens });
    }

    fn reject_last(&mut self, project: &str) {
        let Some(last) = self.last.remove(project) else { return };
        let Some(t) = self.by.get_mut(&last.key) else { return };
        match last.acceptance {
            Acceptance::Rejected(_) => {}
            Acceptance::Accepted => {
                t.accepted = t.accepted.saturating_sub(1);
                t.rejected += 1;
            }
            Acceptance::Unverified => {
                t.unverified = t.unverified.saturating_sub(1);
                t.rejected += 1;
                t.checked_tokens += last.spent_tokens;
            }
        }
    }

    /// Every (model, task type) with its tally.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &Tally)> {
        self.by.iter().map(|((m, t), tally)| (m.as_str(), t.as_str(), tally))
    }

    /// One model's tally over all its task types.
    pub fn by_model(&self) -> BTreeMap<&str, Tally> {
        let mut out: BTreeMap<&str, Tally> = BTreeMap::new();
        for ((m, _), t) in &self.by {
            out.entry(m.as_str()).or_default().merge(t);
        }
        out
    }

    /// The part routing reads: models whose provider and task type are known.
    pub fn records(&self) -> TrackRecords {
        let mut out = TrackRecords::default();
        for ((model, task), t) in &self.by {
            let Some((provider, id)) = model.split_once(" · ") else { continue };
            let (Ok(provider), Ok(task)) = (provider.parse::<ProviderId>(), task.parse::<TaskType>()) else { continue };
            out.insert(provider, id, task, TrackRecord { verified: t.verified(), accepted: t.accepted, tokens_per_accepted: t.tokens_per_accepted() });
        }
        out
    }
}

#[cfg(test)]
mod tests;
