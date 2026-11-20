//! Where start-up time goes. The session start is split into named phases ([`Phase`]); a
//! [`Timeline`] records when each began and how long it took, and renders the `--profile`
//! report and the `--trace` lines. Each phase has a [`Phase::budget`]: the report marks a
//! phase that is over it, and the perf-budget tests fail on it, so a slow change shows up
//! as a failing test instead of as a sluggish prompt a month later.

use crate::providers::domain::probe_source::ProbeSource;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// A step of the session start-up, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The Jev client from the environment and `~/.jev-router.env`.
    JevClient,
    /// Which providers can be used: probed live, or read from the cache.
    ProviderProbe,
    /// The saved preferences of the last session (mode, routing, orchestration).
    SavedState,
    /// The engine, the router and the REPL assembled.
    Engine,
    /// The banner, the resume offer and the hints.
    Banner,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::JevClient => "jev client",
            Self::ProviderProbe => "provider probes",
            Self::SavedState => "saved state",
            Self::Engine => "engine",
            Self::Banner => "banner",
        }
    }

    /// The longest this phase may take on a healthy machine. Provider probes are budgeted
    /// for a cache hit; a cold probe runs the provider CLIs and is allowed to take longer
    /// (see [`ProbeSource`]).
    pub fn budget(self, probes: ProbeSource) -> Duration {
        match (self, probes) {
            (Self::ProviderProbe, ProbeSource::Live) => Duration::from_secs(5),
            (Self::ProviderProbe, ProbeSource::Cache) => Duration::from_millis(50),
            (Self::JevClient | Self::SavedState, _) => Duration::from_millis(30),
            (Self::Engine, _) => Duration::from_millis(80),
            (Self::Banner, _) => Duration::from_millis(60),
        }
    }
}

/// One timed phase: when it began (from the start of the timeline) and how long it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub phase: Phase,
    pub began: Duration,
    pub took: Duration,
}

/// The phases of one start-up, recorded as they happen.
#[derive(Debug)]
pub struct Timeline {
    origin: Instant,
    spans: Vec<Span>,
    probes: ProbeSource,
    /// Print each phase to stderr as it ends (`--trace`).
    live_trace: bool,
}

impl Timeline {
    pub fn start(live_trace: bool) -> Self {
        Self { origin: Instant::now(), spans: Vec::new(), probes: ProbeSource::Live, live_trace }
    }

    /// Time `work` as `phase`.
    pub fn time<T>(&mut self, phase: Phase, work: impl FnOnce() -> T) -> T {
        let began = self.origin.elapsed();
        let out = work();
        self.record(phase, began);
        out
    }

    /// Record `phase` as running from `began` (an offset on this timeline) until now.
    pub fn record(&mut self, phase: Phase, began: Duration) {
        let span = Span { phase, began, took: self.origin.elapsed().saturating_sub(began) };
        if self.live_trace {
            eprintln!("{}", trace_line(&span));
        }
        self.spans.push(span);
    }

    /// The offset now, for [`Timeline::record`] around an `await`.
    pub fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    pub fn set_probe_source(&mut self, source: ProbeSource) {
        self.probes = source;
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn total(&self) -> Duration {
        self.spans.iter().map(|s| s.took).sum()
    }

    /// The phases that took longer than their budget.
    pub fn over_budget(&self) -> Vec<Span> {
        self.spans.iter().copied().filter(|s| s.took > s.phase.budget(self.probes)).collect()
    }

    /// The `--profile` report.
    pub fn report(&self) -> String {
        let mut out = format!("start-up profile: {} in total (providers {})\n", millis(self.total()), self.probes.label());
        for span in &self.spans {
            let budget = span.phase.budget(self.probes);
            let flag = if span.took > budget { format!("  over budget ({})", millis(budget)) } else { String::new() };
            let _ = writeln!(out, "  {:<16}{:>10}{flag}", span.phase.label(), millis(span.took));
        }
        let over = self.over_budget();
        if over.is_empty() {
            out.push_str("every phase is within its budget\n");
        } else {
            let names: Vec<&str> = over.iter().map(|s| s.phase.label()).collect();
            let _ = writeln!(out, "over budget: {}", names.join(", "));
        }
        out
    }
}

/// `trace +12.0 ms  engine  4.1 ms`, one line of the `--trace` timeline.
pub fn trace_line(span: &Span) -> String {
    format!("trace +{:<9}{:<16}{}", millis(span.began), span.phase.label(), millis(span.took))
}

fn millis(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_with(spans: &[(Phase, u64)], probes: ProbeSource) -> Timeline {
        let mut t = Timeline::start(false);
        t.probes = probes;
        t.spans = spans.iter().map(|(phase, ms)| Span { phase: *phase, began: Duration::ZERO, took: Duration::from_millis(*ms) }).collect();
        t
    }

    #[test]
    fn a_timed_phase_is_recorded_with_its_result_passed_through() {
        let mut t = Timeline::start(false);
        let v = t.time(Phase::Engine, || {
            std::thread::sleep(Duration::from_millis(5));
            7
        });
        assert_eq!(v, 7);
        assert_eq!(t.spans().len(), 1);
        assert!(t.spans()[0].took >= Duration::from_millis(5));
    }

    #[test]
    fn the_report_lists_every_phase_and_the_total() {
        let t = timeline_with(&[(Phase::JevClient, 1), (Phase::ProviderProbe, 20), (Phase::Banner, 4)], ProbeSource::Cache);
        let r = t.report();
        assert!(r.contains("25.0 ms in total") && r.contains("providers cached"), "{r}");
        for p in [Phase::JevClient, Phase::ProviderProbe, Phase::Banner] {
            assert!(r.contains(p.label()), "{r}");
        }
        assert!(r.contains("within its budget"), "{r}");
    }

    #[test]
    fn a_phase_over_its_budget_is_named() {
        let t = timeline_with(&[(Phase::Engine, 500), (Phase::Banner, 1)], ProbeSource::Cache);
        assert_eq!(t.over_budget().len(), 1);
        let r = t.report();
        assert!(r.contains("over budget: engine"), "{r}");
    }

    #[test]
    fn the_probe_budget_depends_on_where_the_answer_came_from() {
        let slow = [(Phase::ProviderProbe, 900)];
        assert_eq!(timeline_with(&slow, ProbeSource::Cache).over_budget().len(), 1, "a cache read must be quick");
        assert!(timeline_with(&slow, ProbeSource::Live).over_budget().is_empty(), "a live probe runs the CLIs");
    }

    #[test]
    fn a_trace_line_has_the_offset_the_phase_and_the_duration() {
        let line = trace_line(&Span { phase: Phase::Banner, began: Duration::from_millis(12), took: Duration::from_millis(3) });
        assert!(line.starts_with("trace +12.0 ms") && line.contains("banner") && line.ends_with("3.0 ms"), "{line}");
    }
}
