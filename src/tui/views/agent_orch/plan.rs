//! The plan checklist and the approve-plan card.

use crate::tui::render::Style;
use crate::tui::views::agent_render::{clamp_width, fit, plural};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

impl StepState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Pending => "☐",
            Self::Running => "▶",
            Self::Done => "☑",
            Self::Failed => "✗",
            Self::Skipped => "−",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub title: String,
    /// Indexes of the steps this one waits for.
    pub after: Vec<usize>,
    pub state: StepState,
}

impl PlanStep {
    pub fn new(title: &str, after: &[usize], state: StepState) -> Self {
        Self { title: title.into(), after: after.to_vec(), state }
    }
}

/// The wave each step runs in: 0 with no dependencies, else one past its latest
/// dependency. Steps in one wave run in parallel. A dependency on a later or unknown step
/// is ignored, so a malformed plan still renders.
pub fn waves(steps: &[PlanStep]) -> Vec<usize> {
    let mut wave = vec![0usize; steps.len()];
    for i in 0..steps.len() {
        wave[i] = steps[i].after.iter().filter(|&&d| d < i).map(|&d| wave[d] + 1).max().unwrap_or(0);
    }
    wave
}

/// The plan as a checklist grouped by wave; collapses to one line once every step is
/// settled and `collapse_when_done` is set.
pub fn plan_view(steps: &[PlanStep], collapse_when_done: bool, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let settled = |s: &PlanStep| matches!(s.state, StepState::Done | StepState::Skipped);
    let done = steps.iter().filter(|s| settled(s)).count();
    if collapse_when_done && !steps.is_empty() && done == steps.len() {
        return style.dim(&format!("  ☑ plan: {} done", plural(steps.len(), "step", "steps")));
    }
    let wave = waves(steps);
    let mut out = vec![format!("  {}  {}", style.bold("Plan"), style.dim(&format!("{done} of {}", steps.len())))];
    let last = wave.iter().copied().max().unwrap_or(0);
    for w in 0..=last {
        let in_wave: Vec<usize> = (0..steps.len()).filter(|&i| wave[i] == w).collect();
        if in_wave.len() > 1 {
            out.push(format!("    {}", style.dim(&format!("in parallel ({}):", in_wave.len()))));
        }
        for i in in_wave {
            let s = &steps[i];
            let deps = if s.after.is_empty() { String::new() } else { format!(" (after {})", s.after.iter().map(|d| (d + 1).to_string()).collect::<Vec<_>>().join(", ")) };
            let line = fit(&format!("{} {}. {}{deps}", s.state.glyph(), i + 1, s.title), width.saturating_sub(6));
            out.push(format!("    {}", match s.state {
                StepState::Done | StepState::Skipped => style.dim(&line),
                StepState::Failed => style.red(&line),
                StepState::Running => style.bold(&line),
                StepState::Pending => line,
            }));
        }
    }
    out.join("\n")
}

/// The card that asks before the planner delegates.
pub fn approve_plan_card(steps: &[PlanStep], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  {}", style.bold("Approve this plan before workers start?")), plan_view(steps, false, width, style)];
    out.push(format!("  {}", style.bold("1. Approve   2. Edit the plan   3. Skip a step   4. Cancel")));
    out.join("\n")
}
