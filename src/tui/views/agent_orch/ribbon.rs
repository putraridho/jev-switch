//! The phase ribbon and the role badges.

use crate::tui::render::{human_duration, Style, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Plan,
    Delegate,
    Review,
    Report,
}

impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Plan, Phase::Delegate, Phase::Review, Phase::Report];

    fn name(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Delegate => "delegate",
            Self::Review => "review",
            Self::Report => "report",
        }
    }
}

/// `plan ✓ 4s → [delegate] 12s → review → report`: the active phase in brackets (and
/// bold), finished ones ticked with their time. `elapsed_ms` is per phase, in order, for
/// the phases that have started.
pub fn phase_ribbon(active: Phase, elapsed_ms: &[u64], style: Style) -> String {
    let at = Phase::ALL.iter().position(|p| *p == active).unwrap_or(0);
    let parts: Vec<String> = Phase::ALL
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let time = elapsed_ms.get(i).filter(|_| i <= at).map(|ms| format!(" {}", human_duration(*ms))).unwrap_or_default();
            match i.cmp(&at) {
                std::cmp::Ordering::Less => style.green(&format!("{} ✓{time}", p.name())),
                std::cmp::Ordering::Equal => style.bold(&format!("[{}]{time}", p.name())),
                std::cmp::Ordering::Greater => style.dim(p.name()),
            }
        })
        .collect();
    format!("  {}", parts.join(&style.dim(" → ")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Planner,
    Worker,
    Reviewer,
}

/// `[planner] claude opus (top)`: the role as a word in brackets, then the model in its
/// provider colour with its tier.
pub fn role_badge(role: Role, provider: &str, model: &str, tier: &str, style: Style) -> String {
    let name = match role {
        Role::Planner => "planner",
        Role::Worker => "worker",
        Role::Reviewer => "reviewer",
    };
    let badge = match role {
        Role::Planner => style.bold(&format!("[{name}]")),
        Role::Worker => style.dim(&format!("[{name}]")),
        Role::Reviewer => style.yellow(&format!("[{name}]")),
    };
    format!("{badge} {} {}", Theme::new(style).provider(provider, &format!("{provider} {model}")), style.dim(&format!("({tier})")))
}
