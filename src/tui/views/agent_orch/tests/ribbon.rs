use super::*;
use crate::tui::render::strip_ansi;
use crate::tui::views::agent_orch::ribbon::*;

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
