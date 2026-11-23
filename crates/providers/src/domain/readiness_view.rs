//! A provider's ladder in words: the rung reached, when it was last proven, when it is tried
//! again, and what to do about a rung that failed. Shared by `/providers` and `doctor`.

use jev_backend::ProviderId;
use jev_shared::domain::duration_text::format_left;

use super::breaker::Admission;
use super::readiness::{fix_steps, Ladder, Rung, RungState};

/// When a provider is tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Nothing is waiting.
    Now,
    /// At this time (epoch ms): a breaker's trial attempt, or a hold's expiry.
    At(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessView {
    pub provider: ProviderId,
    /// One line: rung reached, last evidence, what failed, when it is retried.
    pub line: String,
    /// The rung that failed and why, if one did.
    pub failed: Option<(Rung, String)>,
    pub retry: Retry,
    /// What to do about the failed rung; empty when none failed.
    pub fix: Vec<String>,
}

fn ago(now_ms: u64, at_ms: u64) -> String {
    format!("{} ago", format_left(now_ms.saturating_sub(at_ms)))
}

/// Describe `ladder` for `provider`, with its breaker's state.
pub fn describe(provider: ProviderId, ladder: &Ladder, breaker: Admission, now_ms: u64) -> ReadinessView {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match (ladder.reached(), ladder.last_evidence_ms()) {
        (Some(rung), Some(at)) => format!("reached {} (last evidence {})", rung.label(), ago(now_ms, at)),
        (Some(rung), None) => format!("reached {}", rung.label()),
        (None, _) => "nothing proven".to_string(),
    });
    let mut retry = Retry::Now;
    let mut fix = Vec::new();
    let failed = ladder.failure().map(|(rung, f)| (rung, f.what.clone()));
    if let Some((rung, f)) = ladder.failure() {
        if f.until_ms > 0 {
            retry = Retry::At(f.until_ms);
            parts.push(format!(
                "{} failed: {}; held until the CLI, login, backend or folder changes (or {})",
                rung.label(),
                f.what,
                format_left(f.until_ms.saturating_sub(now_ms))
            ));
        } else {
            parts.push(format!("{} failed: {}", rung.label(), f.what));
        }
        fix = fix_steps(provider, rung);
    }
    match breaker {
        Admission::Closed => {}
        Admission::Open { until_ms } => {
            retry = Retry::At(until_ms);
            parts.push(format!("breaker open; one trial attempt in {}", format_left(until_ms.saturating_sub(now_ms))));
        }
        Admission::HalfOpen => parts.push("breaker half-open; the next attempt is the trial".to_string()),
        Admission::Trial { until_ms } => {
            retry = Retry::At(until_ms);
            parts.push("breaker half-open; a trial attempt is running".to_string());
        }
    }
    let unverified: Vec<&str> = ladder.rows.iter().filter(|(_, s)| matches!(s, RungState::Unverified)).map(|(r, _)| r.label()).collect();
    if !unverified.is_empty() {
        parts.push(format!("unverified: {}", unverified.join(", ")));
    }
    ReadinessView { provider, line: format!("{provider}: {}", parts.join("; ")), failed, retry, fix }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::availability::{Availability, State};
    use crate::domain::readiness::{classify, Readiness, ReadinessKey};
    use jev_backend::ErrorKind;

    const NOW: u64 = 1_760_000_000_000;

    fn ready() -> Availability {
        Availability { provider: ProviderId::Gemini, state: State::Ready, plan: None, models: None, detail: "agy login".into(), verified: false }
    }

    #[test]
    fn a_healthy_ladder_names_the_rung_reached_and_what_is_unverified() {
        let v = describe(ProviderId::Gemini, &Ladder::build(&ready(), NOW - 300_000, None, NOW), Admission::Closed, NOW);
        assert!(v.line.starts_with("gemini: reached logged in (last evidence 5m ago)"), "{}", v.line);
        assert!(v.line.contains("unverified: entitled, protocol-compatible, validated for this project"), "{}", v.line);
        assert!(v.failed.is_none() && v.fix.is_empty() && v.retry == Retry::Now);
    }

    #[test]
    fn a_held_provider_says_what_failed_when_it_retries_and_how_to_fix_it() {
        let (mut r, k) = (Readiness::default(), ReadinessKey::default());
        r.fail(ProviderId::Gemini, "p", &k, classify(&ErrorKind::Unsupported).unwrap(), "retired client", NOW);
        let ladder = Ladder::build(&ready(), NOW, r.record(ProviderId::Gemini, "p", &k), NOW + 60_000);
        let v = describe(ProviderId::Gemini, &ladder, Admission::Open { until_ms: NOW + 600_000 }, NOW + 60_000);
        assert!(v.line.contains("protocol-compatible failed: retired client; held until the CLI, login, backend or folder changes (or 23h 59m)"), "{}", v.line);
        assert!(v.line.contains("breaker open; one trial attempt in 9m"), "{}", v.line);
        assert_eq!(v.failed.map(|f| f.0), Some(Rung::ProtocolCompatible));
        assert!(v.fix.join(" ").contains("Update the CLI"));
        assert_eq!(v.retry, Retry::At(NOW + 600_000));
    }
}
