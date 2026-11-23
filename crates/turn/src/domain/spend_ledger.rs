//! What every process of one jev-code session spent, in one place: the planner's own turns
//! and the workers its delegate tool starts (a separate process) write the usage of each
//! finished attempt to a shared ledger, so a paid-use cap applies to the total instead of to
//! each process alone.
//!
//! A session is told apart from older ones by a marker the planner writes when it starts.
//! Workers do not know their planner, so they count from the newest marker of their project.

use jev_backend::{ProviderId, Usage};
use super::budget::{Meter, PriceTable};

/// How far back the spend of a project with no session marker counts, in ms.
pub const NO_MARKER_WINDOW_MS: u64 = 6 * 60 * 60 * 1000;

/// One line of the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerLine {
    /// A planner session of `project` started.
    Session { ts_ms: u64, project: String },
    /// One finished attempt, by `writer` (one process).
    Spend { ts_ms: u64, project: String, writer: String, provider: ProviderId, model: String, usage: Usage },
}

/// When the session of `project` that counts now started: the newest marker, else
/// [`NO_MARKER_WINDOW_MS`] before the project's newest line (so a marker lost to rotation
/// never makes old spend count forever).
pub fn session_start(lines: &[LedgerLine], project: &str) -> u64 {
    let mut newest_marker: Option<u64> = None;
    let mut newest_line = 0;
    for line in lines {
        match line {
            LedgerLine::Session { ts_ms, project: p } if p == project => {
                newest_marker = newest_marker.max(Some(*ts_ms));
                newest_line = newest_line.max(*ts_ms);
            }
            LedgerLine::Spend { ts_ms, project: p, .. } if p == project => newest_line = newest_line.max(*ts_ms),
            _ => {}
        }
    }
    newest_marker.unwrap_or_else(|| newest_line.saturating_sub(NO_MARKER_WINDOW_MS))
}

/// What the writers other than `me` spent in `project` since `since_ms`, as a meter (so
/// unreported and unpriced attempts count the way they do for the process's own spend).
pub fn others_spend(lines: &[LedgerLine], project: &str, since_ms: u64, me: &str, prices: &PriceTable) -> Meter {
    let mut meter = Meter::default();
    for line in lines {
        if let LedgerLine::Spend { ts_ms, project: p, writer, provider, model, usage } = line {
            if p == project && writer != me && *ts_ms >= since_ms {
                meter.record(*provider, model, usage, prices);
            }
        }
    }
    meter
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::Measurement;

    fn spend(ts: u64, project: &str, writer: &str, tokens: u64) -> LedgerLine {
        LedgerLine::Spend {
            ts_ms: ts,
            project: project.into(),
            writer: writer.into(),
            provider: ProviderId::DeepSeek,
            model: "deepseek-flash".into(),
            usage: Usage { input_tokens: tokens, ..Usage::default() },
        }
    }

    #[test]
    fn the_newest_marker_of_the_project_starts_the_session() {
        let lines = [
            LedgerLine::Session { ts_ms: 100, project: "/a".into() },
            LedgerLine::Session { ts_ms: 500, project: "/b".into() },
            LedgerLine::Session { ts_ms: 300, project: "/a".into() },
        ];
        assert_eq!(session_start(&lines, "/a"), 300);
        assert_eq!(session_start(&lines, "/c"), 0, "nothing at all");
        let only_spend = [spend(NO_MARKER_WINDOW_MS + 7, "/c", "w", 1)];
        assert_eq!(session_start(&only_spend, "/c"), 7, "no marker: a bounded look back from the newest line");
    }

    #[test]
    fn others_spend_counts_other_writers_of_this_project_since_the_start_only() {
        let lines = [
            spend(50, "/a", "w1", 1_000),
            spend(150, "/a", "w1", 10),
            spend(160, "/a", "me", 5_000),
            spend(170, "/b", "w2", 7_000),
            spend(180, "/a", "w2", 20),
        ];
        let meter = others_spend(&lines, "/a", 100, "me", &PriceTable::default());
        assert_eq!(meter.paid().1, 30);
    }

    #[test]
    fn unreported_attempts_stay_unreported_so_a_cap_still_fails_closed() {
        let mut line = spend(150, "/a", "w1", 0);
        if let LedgerLine::Spend { usage, .. } = &mut line {
            usage.measurement = Measurement::Unavailable;
        }
        let meter = others_spend(&[line], "/a", 0, "me", &PriceTable::default());
        assert_eq!(meter.unreported_paid_turns(), 1);
    }
}
