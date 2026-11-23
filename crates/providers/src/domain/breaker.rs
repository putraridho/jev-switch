//! A circuit breaker per provider: a provider that keeps failing in a way that is not a
//! usage limit (retired client, empty or stalled answers) is paused for a while, so a new
//! session does not rediscover the same breakage turn by turn. The pause expires and doubles
//! each time the provider fails again right after it, and any success ends it.
//!
//! Only failures that say something about the provider itself count. A login problem (the
//! user fixes it), a directory refusal (it depends on the folder) and a limit (a cooldown
//! covers it) do not.

use std::collections::HashMap;

use jev_shared::ProviderId;

/// Consecutive counted failures that open the breaker.
pub const TRIP_AFTER: u32 = 4;
/// The first pause, in ms; each further trip without a success doubles it.
pub const FIRST_PAUSE_MS: u64 = 10 * 60 * 1000;
/// The longest pause, in ms.
pub const LONGEST_PAUSE_MS: u64 = 2 * 60 * 60 * 1000;

/// One provider's record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Breaker {
    /// Counted failures since the last success.
    pub failures: u32,
    /// Times the breaker opened since the last success.
    pub trips: u32,
    /// The pause ends at this time (epoch ms); 0 when it never opened.
    pub open_until_ms: u64,
    /// What the last counted failure was, in the scoreboard's words.
    pub last: String,
    /// When this record last changed (epoch ms): the newer copy wins a merge.
    pub updated_ms: u64,
}

/// The records of every provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakers {
    by_provider: HashMap<ProviderId, Breaker>,
}

/// The pause after the `trips`-th opening.
fn pause_ms(trips: u32) -> u64 {
    FIRST_PAUSE_MS.saturating_mul(1u64 << trips.saturating_sub(1).min(20)).min(LONGEST_PAUSE_MS)
}

impl Breakers {
    /// Count one failure. Once [`TRIP_AFTER`] in a row are counted the breaker opens, and
    /// because the count is kept until a success, the first failure after a pause ends it
    /// opens it again at once, for twice as long.
    pub fn record_failure(&mut self, p: ProviderId, what: &str, now_ms: u64) {
        let b = self.by_provider.entry(p).or_default();
        b.failures = b.failures.saturating_add(1);
        b.last = what.to_string();
        b.updated_ms = now_ms;
        if b.failures >= TRIP_AFTER {
            b.trips = b.trips.saturating_add(1);
            b.open_until_ms = now_ms.saturating_add(pause_ms(b.trips));
        }
    }

    /// A provider answered: its record is wiped. Returns whether there was one to wipe.
    /// The wiped record stays as a clean one stamped `now_ms`, so merging with a saved copy
    /// cannot bring the old failures back.
    pub fn record_success(&mut self, p: ProviderId, now_ms: u64) -> bool {
        let had = self.by_provider.get(&p).is_some_and(|b| b.failures > 0);
        if had {
            self.by_provider.insert(p, Breaker { updated_ms: now_ms, ..Breaker::default() });
        }
        had
    }

    /// Wipe every record (`/providers refresh`: the user asks to try again). Returns whether
    /// there was anything to wipe.
    pub fn clear(&mut self, now_ms: u64) -> bool {
        let dirty: Vec<ProviderId> = self.by_provider.iter().filter(|(_, b)| b.failures > 0).map(|(p, _)| *p).collect();
        for p in &dirty {
            self.record_success(*p, now_ms);
        }
        !dirty.is_empty()
    }

    /// The open pause of `p`, if any.
    pub fn open(&self, p: ProviderId, now_ms: u64) -> Option<&Breaker> {
        self.by_provider.get(&p).filter(|b| b.open_until_ms > now_ms)
    }

    /// Per provider, the record that changed last.
    pub fn merged(&self, other: &Self) -> Self {
        let mut by_provider = self.by_provider.clone();
        for (p, theirs) in &other.by_provider {
            match by_provider.get(p) {
                Some(mine) if mine.updated_ms >= theirs.updated_ms => {}
                _ => {
                    by_provider.insert(*p, theirs.clone());
                }
            }
        }
        Self { by_provider }
    }

    pub fn entries(&self) -> &HashMap<ProviderId, Breaker> {
        &self.by_provider
    }
}

impl From<HashMap<ProviderId, Breaker>> for Breakers {
    fn from(by_provider: HashMap<ProviderId, Breaker>) -> Self {
        Self { by_provider }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: ProviderId = ProviderId::Gemini;

    fn failed(b: &mut Breakers, times: u32, now: u64) {
        for _ in 0..times {
            b.record_failure(P, "empty answer", now);
        }
    }

    #[test]
    fn it_opens_only_after_enough_failures_in_a_row() {
        let mut b = Breakers::default();
        failed(&mut b, TRIP_AFTER - 1, 1_000);
        assert!(b.open(P, 1_000).is_none());
        failed(&mut b, 1, 1_000);
        assert!(b.open(P, 1_000).is_some());
        assert!(b.open(ProviderId::Claude, 1_000).is_none());
    }

    #[test]
    fn the_pause_ends_by_itself() {
        let mut b = Breakers::default();
        failed(&mut b, TRIP_AFTER, 1_000);
        assert!(b.open(P, 1_000 + FIRST_PAUSE_MS - 1).is_some());
        assert!(b.open(P, 1_000 + FIRST_PAUSE_MS).is_none());
    }

    #[test]
    fn a_failure_right_after_a_pause_reopens_it_for_twice_as_long_up_to_a_cap() {
        let mut b = Breakers::default();
        failed(&mut b, TRIP_AFTER, 0);
        let mut now = FIRST_PAUSE_MS;
        for trips in 2..=12u32 {
            failed(&mut b, 1, now);
            let until = b.entries()[&P].open_until_ms;
            assert_eq!(until - now, (FIRST_PAUSE_MS << (trips - 1)).min(LONGEST_PAUSE_MS), "trip {trips}");
            now = until;
        }
    }

    #[test]
    fn a_success_wipes_the_record_and_a_long_run_of_failures_does_not_overflow() {
        let mut b = Breakers::default();
        failed(&mut b, TRIP_AFTER, 0);
        assert!(b.record_success(P, 7));
        assert!(b.open(P, 1).is_none());
        assert!(!b.record_success(P, 8), "nothing left to wipe");
        failed(&mut b, 200, 5);
        assert_eq!(b.entries()[&P].open_until_ms, 5 + LONGEST_PAUSE_MS);
    }

    #[test]
    fn merging_keeps_the_newer_record_per_provider() {
        let (mut mine, mut theirs) = (Breakers::default(), Breakers::default());
        failed(&mut mine, 1, 10);
        failed(&mut theirs, TRIP_AFTER, 20);
        mine.record_failure(ProviderId::Codex, "stalled", 30);
        let merged = mine.merged(&theirs);
        assert_eq!(merged.entries()[&P].updated_ms, 20);
        assert_eq!(merged.entries()[&ProviderId::Codex].last, "stalled");
        assert_eq!(merged, theirs.merged(&mine));
    }

    #[test]
    fn a_wipe_is_not_undone_by_merging_with_an_older_saved_copy() {
        let mut saved = Breakers::default();
        failed(&mut saved, TRIP_AFTER, 100);
        let mut mine = saved.clone();
        assert!(mine.clear(200));
        assert!(mine.merged(&saved).open(P, 300).is_none());
        assert!(!mine.clear(300), "already clean");
    }
}
