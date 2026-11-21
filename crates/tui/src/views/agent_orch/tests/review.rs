use super::*;
use crate::views::agent_orch::review::*;

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
