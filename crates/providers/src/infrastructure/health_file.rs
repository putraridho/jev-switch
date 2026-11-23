//! The saved circuit breakers: one small JSON file shared by every `jev-code` and
//! `delegate_mcp` process, written atomically and merged with what is already there.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::breaker::{Breaker, Breakers};
use jev_shared::ProviderId;

#[derive(Serialize, Deserialize)]
struct BreakerWire {
    failures: u32,
    trips: u32,
    open_until_ms: u64,
    #[serde(default)]
    last: String,
    #[serde(default)]
    trial_until_ms: u64,
    updated_ms: u64,
}

impl From<&Breaker> for BreakerWire {
    fn from(b: &Breaker) -> Self {
        Self { failures: b.failures, trips: b.trips, open_until_ms: b.open_until_ms, last: b.last.clone(), trial_until_ms: b.trial_until_ms, updated_ms: b.updated_ms }
    }
}

impl From<BreakerWire> for Breaker {
    fn from(w: BreakerWire) -> Self {
        Self { failures: w.failures, trips: w.trips, open_until_ms: w.open_until_ms, last: w.last, trial_until_ms: w.trial_until_ms, updated_ms: w.updated_ms }
    }
}

/// What is saved; a missing, unreadable or garbled file, and entries that are not a known
/// provider or not a record, count as nothing saved.
pub fn load(path: &Path) -> Breakers {
    let Some(text) = std::fs::read_to_string(path).ok() else { return Breakers::default() };
    let Ok(raw) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&text) else { return Breakers::default() };
    raw.into_iter()
        .filter_map(|(name, value)| Some((name.parse::<ProviderId>().ok()?, serde_json::from_value::<BreakerWire>(value).ok()?.into())))
        .collect::<HashMap<_, _>>()
        .into()
}

/// Merge `breakers` with what is saved (the record that changed last wins per provider),
/// write the result, and return it.
pub fn save(path: &Path, breakers: &Breakers) -> std::io::Result<Breakers> {
    let merged = breakers.merged(&load(path));
    let wire: HashMap<&str, BreakerWire> = merged.entries().iter().map(|(p, b)| (p.as_str(), BreakerWire::from(b))).collect();
    super::atomic_file::write_atomic(path, &serde_json::to_string(&wire)?)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        jev_testkit::scratch_dir::scratch_dir("health", name)
    }

    fn open_gemini(now: u64) -> Breakers {
        let mut b = Breakers::default();
        for _ in 0..crate::domain::breaker::TRIP_AFTER {
            b.record_failure(ProviderId::Gemini, "empty answer", now);
        }
        b
    }

    #[test]
    fn it_round_trips_and_a_missing_or_garbled_file_is_nothing() {
        let d = dir("round");
        let path = d.join("provider-health.json");
        assert_eq!(load(&path), Breakers::default());
        let b = open_gemini(1_000);
        assert_eq!(save(&path, &b).unwrap(), b);
        assert_eq!(load(&path), b);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), Breakers::default());
        std::fs::write(&path, r#"{"nobody":{"failures":1},"gemini":"x","claude":{"failures":1,"trips":0,"open_until_ms":0,"updated_ms":5}}"#).unwrap();
        assert_eq!(load(&path).entries().len(), 1);
    }

    #[test]
    fn saving_keeps_what_another_process_saved_and_a_wipe_stays_wiped() {
        let d = dir("merge");
        let path = d.join("provider-health.json");
        save(&path, &open_gemini(1_000)).unwrap();

        let mut other = Breakers::default();
        other.record_failure(ProviderId::Codex, "stalled", 2_000);
        let merged = save(&path, &other).unwrap();
        assert_eq!(merged.entries().len(), 2, "gemini from the file, codex from this process");

        let mut wiped = merged.clone();
        assert!(wiped.record_success(ProviderId::Gemini, 3_000));
        let after = save(&path, &wiped).unwrap();
        assert!(after.open(ProviderId::Gemini, 3_001).is_none());
        assert!(load(&path).open(ProviderId::Gemini, 3_001).is_none());
    }
}
