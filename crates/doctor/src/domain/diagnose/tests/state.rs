use super::*;

#[test]
fn credits_note_appears_only_for_ready_non_max_claude_without_opt_in() {
    let with = |plan: Option<&str>, state, allow| {
        let mut i = inputs(vec![Availability { plan: plan.map(String::from), ..av(Claude, state) }]);
        i.allow_credits = allow;
        has_check(&diagnose(&i), "credits")
    };
    assert!(with(Some("pro"), State::Ready, false));
    assert!(with(None, State::Ready, false));
    assert!(!with(Some("max_5x"), State::Ready, false));
    assert!(!with(Some("pro"), State::Ready, true));
    assert!(!with(Some("pro"), State::NotLoggedIn, false));
    let mut i = inputs(vec![Availability { plan: Some("pro".into()), ..av(Claude, State::Ready) }]);
    i.allow_credits = false;
    let r = diagnose(&i);
    let c = find(&r, "credits");
    assert_eq!(c.severity, Severity::Info);
    assert_eq!(c.fix, vec!["export JEV_ALLOW_CREDITS=1".to_string()]);
}

#[test]
fn cooldowns_become_one_info_check_with_the_durations() {
    let mut i = inputs(vec![]);
    i.cooldown_lines = vec!["claude cooling for 12m".into(), "codex cooling for 3h 5m".into()];
    let r = diagnose(&i);
    let c = find(&r, "cooldowns");
    assert_eq!(c.severity, Severity::Info);
    assert!(c.detail.contains("claude cooling for 12m") && c.detail.contains("codex cooling for 3h 5m"));
    assert!(!has_check(&diagnose(&inputs(vec![])), "cooldowns"));
}
