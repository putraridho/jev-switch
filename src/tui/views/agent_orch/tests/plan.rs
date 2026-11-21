use super::*;
use crate::tui::views::agent_orch::plan::*;

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
fn approve_card_lists_the_plan_and_the_four_choices() {
    let c = approve_plan_card(&plan(), 80, OFF);
    assert!(c.contains("Approve this plan") && c.contains("2. lexer") && c.contains("1. Approve   2. Edit the plan   3. Skip a step   4. Cancel"));
}
