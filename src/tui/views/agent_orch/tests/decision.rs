use super::*;
use crate::orchestration::domain::orchestration_gate::Gate;
use crate::tui::views::agent_orch::decision::*;

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
