//! A scoreboard of how each provider has behaved in this session: answers, failures and
//! how long answers took. Only ever shows what happened; the router does not read it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::backend::{ErrorKind, ProviderId};

/// Latencies kept per provider; older ones fall off.
const KEEP: usize = 50;

#[derive(Debug, Clone, Default)]
pub struct Health {
    pub answered: u32,
    pub failed: u32,
    /// Wall time of recent attempts, answered or failed, oldest first.
    latencies_ms: Vec<u64>,
    last_failure: Option<&'static str>,
}

impl Health {
    /// The median of the kept latencies.
    pub fn median_ms(&self) -> Option<u64> {
        let mut v = self.latencies_ms.clone();
        v.sort_unstable();
        v.get(v.len() / 2).copied()
    }

    fn push(&mut self, ms: u64) {
        self.latencies_ms.push(ms);
        if self.latencies_ms.len() > KEEP {
            self.latencies_ms.remove(0);
        }
    }

    /// Share of attempts that answered, `None` before any.
    pub fn success_rate(&self) -> Option<f64> {
        let total = self.answered + self.failed;
        (total > 0).then(|| f64::from(self.answered) / f64::from(total))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Scoreboard {
    by_provider: BTreeMap<ProviderId, Health>,
}

/// A failure in the words of the scoreboard.
pub fn failure_label(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Quota { .. } => "limit",
        ErrorKind::NotEntitled => "not on plan",
        ErrorKind::Auth => "login",
        ErrorKind::Empty => "empty answer",
        ErrorKind::Stalled => "stalled",
        ErrorKind::UnusableHere => "cannot run here",
        ErrorKind::Unsupported => "retired CLI",
        ErrorKind::Other => "error",
    }
}

impl Scoreboard {
    pub fn record_answer(&mut self, provider: ProviderId, wall_ms: u64) {
        let h = self.by_provider.entry(provider).or_default();
        h.answered += 1;
        h.push(wall_ms);
    }

    pub fn record_failure(&mut self, provider: ProviderId, kind: &ErrorKind, wall_ms: u64) {
        let h = self.by_provider.entry(provider).or_default();
        h.failed += 1;
        h.last_failure = Some(failure_label(kind));
        h.push(wall_ms);
    }

    pub fn health(&self, provider: ProviderId) -> Option<&Health> {
        self.by_provider.get(&provider)
    }

    pub fn is_empty(&self) -> bool {
        self.by_provider.is_empty()
    }

    pub fn text(&self) -> String {
        if self.is_empty() {
            return "  No provider has been tried in this session yet.".to_string();
        }
        let mut out = String::new();
        for (p, h) in &self.by_provider {
            let rate = h.success_rate().map_or("-".to_string(), |r| format!("{:.0}%", r * 100.0));
            let median = h.median_ms().map_or("-".to_string(), |ms| format!("{:.1}s", ms as f64 / 1000.0));
            let last = h.last_failure.map_or(String::new(), |l| format!("  last failure: {l}"));
            let _ = writeln!(out, "  {:<9} {:>2} answered  {:>2} failed  {rate:>4} ok  median {median:>6}{last}", p.as_str(), h.answered, h.failed);
        }
        out.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_rates_and_median() {
        let mut s = Scoreboard::default();
        assert!(s.is_empty() && s.text().contains("No provider"));
        for ms in [3_000, 1_000, 2_000] {
            s.record_answer(ProviderId::Claude, ms);
        }
        s.record_failure(ProviderId::Claude, &ErrorKind::Quota { resets_at: None }, 500);
        let h = s.health(ProviderId::Claude).unwrap();
        assert_eq!((h.answered, h.failed), (3, 1));
        assert_eq!(h.success_rate(), Some(0.75));
        assert_eq!(h.median_ms(), Some(2_000));
        assert!(s.health(ProviderId::Grok).is_none());
    }

    #[test]
    fn old_latencies_fall_off() {
        let mut s = Scoreboard::default();
        for _ in 0..KEEP {
            s.record_answer(ProviderId::Codex, 10_000);
        }
        for _ in 0..KEEP {
            s.record_answer(ProviderId::Codex, 1_000);
        }
        assert_eq!(s.health(ProviderId::Codex).unwrap().median_ms(), Some(1_000));
    }

    #[test]
    fn the_screen() {
        let mut s = Scoreboard::default();
        s.record_answer(ProviderId::Claude, 4_200);
        s.record_failure(ProviderId::DeepSeek, &ErrorKind::Auth, 300);
        let t = s.text();
        assert_eq!(
            t,
            "  claude     1 answered   0 failed  100% ok  median   4.2s\n  deepseek   0 answered   1 failed    0% ok  median   0.3s  last failure: login"
        );
    }
}
