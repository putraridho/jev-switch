//! The decision card: why a prompt was or was not orchestrated.

use jev_orchestration::domain::orchestration_gate::Gate;
use crate::render::Style;
use crate::views::agent_render::{clamp_width, fit};

/// Why a prompt was or was not orchestrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Orchestrated,
    BelowCutoff,
    Vetoed,
    SingleModelRoute,
    /// `/orchestrate off`.
    Off,
    /// Orchestration was wanted but no provider can carry the delegate tool.
    NoPlanner,
}

impl From<&Gate> for Decision {
    fn from(g: &Gate) -> Self {
        match g {
            Gate::Trigger => Self::Orchestrated,
            Gate::BelowCutoff => Self::BelowCutoff,
            Gate::Vetoed => Self::Vetoed,
            Gate::SingleModelRoute => Self::SingleModelRoute,
        }
    }
}

impl Decision {
    fn reason(self) -> &'static str {
        match self {
            Self::Orchestrated => "it splits into independent pieces, so a planner hands them to cheaper workers",
            Self::BelowCutoff => "it does not look like separate pieces, so one model does it",
            Self::Vetoed => "its steps depend on each other, so parallel workers would gain nothing",
            Self::SingleModelRoute => "the normal route is not a top model, so one model is cheaper",
            Self::Off => "orchestration is off for this session (/orchestrate on)",
            Self::NoPlanner => "no ready provider can have the delegate tool attached, so it runs as one turn",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionFacts {
    pub decision: Decision,
    pub multi_part: f64,
    pub cutoff: f64,
    /// Whether the normal route would have been a top-tier model.
    pub would_use_opus: bool,
    /// The chosen planner, `provider model`.
    pub planner: Option<String>,
}

/// The decision card: three rows when orchestrated, ONE line when skipped (no extra noise).
pub fn decision_card(d: &DecisionFacts, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let score = format!("multi-part {:.2} {} cutoff {:.2}", d.multi_part, if d.multi_part >= d.cutoff { "≥" } else { "<" }, d.cutoff);
    if d.decision != Decision::Orchestrated {
        return style.dim(&fit(&format!("  ⇉ not orchestrated: {} ({score})", d.decision.reason()), width));
    }
    let planner = d.planner.as_deref().unwrap_or("auto");
    [
        format!("  {}  {}", style.bold("⇉ Orchestrated"), style.dim(&score)),
        format!("    planner: {planner}   would have gone to Opus: {}", if d.would_use_opus { "yes" } else { "no" }),
        format!("    {}", style.dim(&fit(&format!("why: {}", d.decision.reason()), width.saturating_sub(4)))),
    ]
    .join("\n")
}
