//! The review round and the savings meter.

use crate::tui::render::Style;
use crate::tui::views::agent_render::{clamp_width, fit, usd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRound {
    pub round: usize,
    pub of: usize,
    pub checks: Vec<Check>,
    /// Follow-up steps the planner sent back out.
    pub followups: Vec<String>,
}

/// The planner's review round: each check with pass or fail, then the follow-ups.
pub fn review_view(r: &ReviewRound, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let failed = r.checks.iter().filter(|c| !c.passed).count();
    let verdict = if failed == 0 { style.green("all checks pass") } else { style.red(&format!("{failed} failed")) };
    let mut out = vec![format!("  {}  {}", style.bold(&format!("Review round {} of {}", r.round, r.of)), verdict)];
    for c in &r.checks {
        let line = fit(&format!("{} {}", if c.passed { "✓ pass" } else { "✗ FAIL" }, c.name), width.saturating_sub(4));
        out.push(format!("    {}", if c.passed { style.dim(&line) } else { style.red(&line) }));
    }
    for f in &r.followups {
        out.push(format!("    ↳ {}", fit(f, width.saturating_sub(8))));
    }
    out.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spend {
    pub planner_usd: f64,
    pub workers_usd: f64,
    /// What the same work would have cost on one top model.
    pub solo_usd: f64,
}

impl Spend {
    pub fn total(&self) -> f64 {
        self.planner_usd + self.workers_usd
    }

    /// Whole percent saved against the solo estimate; negative when it cost more;
    /// `None` without a baseline.
    pub fn saved_pct(&self) -> Option<i64> {
        (self.solo_usd > 0.0).then(|| ((self.solo_usd - self.total()) / self.solo_usd * 100.0).round() as i64)
    }
}

/// `orchestrated $0.80 (planner $0.50 · workers $0.30) · est. solo Opus $1.40 · saved 43%`.
pub fn savings_meter(s: &Spend, style: Style) -> String {
    let verdict = match s.saved_pct() {
        None => style.dim("no solo estimate"),
        Some(p) if p >= 0 => style.green(&format!("saved {p}%")),
        Some(p) => style.red(&format!("cost {}% more", -p)),
    };
    format!(
        "  orchestrated {} {} · est. solo Opus {} · {verdict}",
        usd(s.total()),
        style.dim(&format!("(planner {} · workers {})", usd(s.planner_usd), usd(s.workers_usd))),
        usd(s.solo_usd)
    )
}
