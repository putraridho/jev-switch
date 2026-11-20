//! The orchestration visuals: decision card, plan, phase ribbon, role badges, review
//! loop, savings meter, flow diagram and the approve-plan card.

use crate::tui::views::agent_workers::{WorkerInfo, WorkerState};
use crate::tui::views::agent_render::{clamp_width, fit, plural, usd};
use crate::orchestration::domain::orchestration_gate::Gate;
use crate::tui::render::{display_width, human_duration, Style, Theme};

// ------------------------------------------------------------ decision card

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

// ------------------------------------------------------------ plan

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

// ------------------------------------------------------------ phases

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

// ------------------------------------------------------------ roles

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

// ------------------------------------------------------------ review loop

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

// ------------------------------------------------------------ savings

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

// ------------------------------------------------------------ flow diagram

/// The whole turn as a diagram: planner at the top, one lane per worker, merge at the
/// bottom.
pub fn flow_diagram(planner: &str, workers: &[WorkerInfo], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  ┌ {}", style.bold(&format!("planner · {}", fit(planner, width.saturating_sub(16)))))];
    if workers.is_empty() {
        out.push(format!("  │ {}", style.dim("(no workers yet)")));
    }
    for w in workers {
        let head = format!("  ├──▶ {} {} {} ", w.state.glyph(), w.id, w.state.word());
        let who = fit(&format!("{} {}", w.provider, w.model), width.saturating_sub(display_width(&head)));
        let room = width.saturating_sub(display_width(&head) + display_width(&who) + 1);
        let task = if room > 3 { format!(" {}", style.dim(&fit(&w.task, room))) } else { String::new() };
        out.push(format!("{head}{}{task}", Theme::new(style).provider(&w.provider, &who)));
    }
    let done = workers.iter().filter(|w| w.state == WorkerState::Done).count();
    out.push(format!("  │ {}", style.dim(&format!("{done} of {} merged back", workers.len()))));
    out.push(format!("  └ {}", style.bold("merge → review → report")));
    out.join("\n")
}

/// The diagram as a Markdown section, for export.
pub fn flow_markdown(planner: &str, workers: &[WorkerInfo]) -> String {
    format!("## Orchestration\n\n```text\n{}\n```\n", flow_diagram(planner, workers, 100, Style::off()))
}

// ------------------------------------------------------------ approve plan

/// The card that asks before the planner delegates.
pub fn approve_plan_card(steps: &[PlanStep], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  {}", style.bold("Approve this plan before workers start?")), plan_view(steps, false, width, style)];
    out.push(format!("  {}", style.bold("1. Approve   2. Edit the plan   3. Skip a step   4. Cancel")));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::strip_ansi;

    const OFF: Style = Style { enabled: false, terminal: false };
    const ON: Style = Style { enabled: true, terminal: true };

    fn facts(d: Decision) -> DecisionFacts {
        DecisionFacts { decision: d, multi_part: 0.82, cutoff: 0.7, would_use_opus: true, planner: Some("claude opus".into()) }
    }

    #[test]
    fn orchestrated_card_has_score_planner_opus_answer_and_reason() {
        let c = decision_card(&facts(Decision::Orchestrated), 100, OFF);
        assert_eq!(c.lines().count(), 3);
        for want in ["multi-part 0.82 ≥ cutoff 0.70", "planner: claude opus", "would have gone to Opus: yes", "why:"] {
            assert!(c.contains(want), "{want}: {c}");
        }
    }

    #[test]
    fn a_skipped_prompt_is_one_line_with_its_reason() {
        let mut f = facts(Decision::BelowCutoff);
        f.multi_part = 0.3;
        let c = decision_card(&f, 120, OFF);
        assert_eq!(c.lines().count(), 1);
        assert!(c.contains("not orchestrated") && c.contains("0.30 < cutoff 0.70") && c.contains("one model does it"), "{c}");
    }

    #[test]
    fn every_gate_outcome_maps_to_a_decision() {
        assert_eq!(Decision::from(&Gate::Trigger), Decision::Orchestrated);
        assert_eq!(Decision::from(&Gate::Vetoed), Decision::Vetoed);
        assert_eq!(Decision::from(&Gate::BelowCutoff), Decision::BelowCutoff);
        assert_eq!(Decision::from(&Gate::SingleModelRoute), Decision::SingleModelRoute);
    }

    fn plan() -> Vec<PlanStep> {
        vec![
            PlanStep::new("parser", &[], StepState::Done),
            PlanStep::new("lexer", &[], StepState::Running),
            PlanStep::new("integrate", &[0, 1], StepState::Pending),
        ]
    }

    #[test]
    fn waves_put_independent_steps_together_and_ignore_bad_dependencies() {
        assert_eq!(waves(&plan()), vec![0, 0, 1]);
        let bad = [PlanStep::new("a", &[5, 0], StepState::Pending), PlanStep::new("b", &[0], StepState::Pending)];
        assert_eq!(waves(&bad), vec![0, 1]);
    }

    #[test]
    fn plan_view_ticks_marks_parallel_steps_and_collapses_when_done() {
        let v = plan_view(&plan(), true, 80, OFF);
        for want in ["1 of 3", "in parallel (2):", "☑ 1. parser", "▶ 2. lexer", "☐ 3. integrate (after 1, 2)"] {
            assert!(v.contains(want), "{want}: {v}");
        }
        let all_done: Vec<PlanStep> = plan().into_iter().map(|mut s| { s.state = StepState::Done; s }).collect();
        assert_eq!(plan_view(&all_done, true, 80, OFF), "  ☑ plan: 3 steps done");
        assert!(plan_view(&all_done, false, 80, OFF).contains("3 of 3"));
    }

    #[test]
    fn ribbon_marks_the_active_phase_and_times_finished_ones() {
        assert_eq!(phase_ribbon(Phase::Delegate, &[4000, 12000], OFF), "  plan ✓ 4s → [delegate] 12s → review → report");
        assert_eq!(phase_ribbon(Phase::Plan, &[], OFF), "  [plan] → delegate → review → report");
        assert_eq!(strip_ansi(&phase_ribbon(Phase::Review, &[1000, 2000, 3000], ON)), phase_ribbon(Phase::Review, &[1000, 2000, 3000], OFF));
    }

    #[test]
    fn badges_name_role_model_and_tier_in_words() {
        assert_eq!(role_badge(Role::Planner, "claude", "opus", "top", OFF), "[planner] claude opus (top)");
        assert_eq!(role_badge(Role::Worker, "codex", "gpt-x", "light", OFF), "[worker] codex gpt-x (light)");
        assert!(role_badge(Role::Reviewer, "claude", "sonnet", "everyday", OFF).starts_with("[reviewer]"));
    }

    #[test]
    fn review_view_shows_round_checks_and_followups() {
        let r = ReviewRound {
            round: 2,
            of: 3,
            checks: vec![Check { name: "cargo test".into(), passed: true }, Check { name: "cargo clippy".into(), passed: false }],
            followups: vec!["fix the lint in lexer.rs".into()],
        };
        let v = review_view(&r, 80, OFF);
        for want in ["Review round 2 of 3", "1 failed", "✓ pass cargo test", "✗ FAIL cargo clippy", "↳ fix the lint"] {
            assert!(v.contains(want), "{want}: {v}");
        }
        let ok = ReviewRound { checks: vec![], followups: vec![], ..r };
        assert!(review_view(&ok, 80, OFF).contains("all checks pass"));
    }

    #[test]
    fn savings_meter_matches_the_spec_example_and_admits_losses() {
        let s = Spend { planner_usd: 0.50, workers_usd: 0.30, solo_usd: 1.40 };
        assert_eq!(s.saved_pct(), Some(43));
        assert_eq!(savings_meter(&s, OFF), "  orchestrated $0.80 (planner $0.50 · workers $0.30) · est. solo Opus $1.40 · saved 43%");
        let worse = Spend { planner_usd: 1.0, workers_usd: 0.5, solo_usd: 1.0 };
        assert!(savings_meter(&worse, OFF).contains("cost 50% more"));
        assert_eq!(Spend { solo_usd: 0.0, ..s }.saved_pct(), None);
    }

    fn workers() -> Vec<WorkerInfo> {
        vec![WorkerInfo::new("w1", "codex", "gpt-x", "parser", WorkerState::Done), WorkerInfo::new("w2", "gemini", "flash", "lexer", WorkerState::Running)]
    }

    #[test]
    fn flow_diagram_has_planner_lanes_and_merge() {
        let d = flow_diagram("claude opus", &workers(), 90, OFF);
        let rows: Vec<&str> = d.lines().collect();
        assert!(rows[0].contains("planner · claude opus"));
        assert!(rows[1].contains("├──▶") && rows[1].contains("w1") && rows[1].contains("done"));
        assert!(rows[2].contains("w2") && rows[2].contains("running"));
        assert!(d.contains("1 of 2 merged back") && d.contains("└ merge → review → report"));
        for l in flow_diagram("claude opus", &workers(), 30, OFF).lines() {
            assert!(display_width(l) <= 30, "{l:?}");
        }
        assert!(flow_diagram("p", &[], 60, OFF).contains("no workers yet"));
    }

    #[test]
    fn markdown_export_fences_the_diagram() {
        let m = flow_markdown("claude opus", &workers());
        assert!(m.starts_with("## Orchestration\n\n```text\n") && m.ends_with("```\n") && !m.contains('\x1b'));
    }

    #[test]
    fn approve_card_lists_the_plan_and_the_four_choices() {
        let c = approve_plan_card(&plan(), 80, OFF);
        assert!(c.contains("Approve this plan") && c.contains("2. lexer") && c.contains("1. Approve   2. Edit the plan   3. Skip a step   4. Cancel"));
    }
}
