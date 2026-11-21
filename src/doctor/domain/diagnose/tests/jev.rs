use super::*;

#[test]
fn missing_jev_key_is_a_problem_with_the_exact_fix_and_skips_reachability() {
    let mut i = inputs(vec![av(Claude, State::Ready)]);
    i.jev_key_present = false;
    i.jev_reachable = None;
    let r = diagnose(&i);
    let c = find(&r, "jev-key");
    assert_eq!(c.severity, Severity::Problem);
    assert_eq!(c.fix, vec!["Add JEV_API_KEY=... to ~/.jev-router.env".to_string()]);
    assert!(!has_check(&r, "jev-reachable"));
}

#[test]
fn jev_reachability_table() {
    for (reachable, expect) in [(Some(true), Some(Severity::Ok)), (Some(false), Some(Severity::Warn)), (None, None)] {
        let mut i = inputs(vec![av(Claude, State::Ready)]);
        i.jev_reachable = reachable;
        let r = diagnose(&i);
        assert_eq!(r.checks.iter().find(|c| c.id == "jev-reachable").map(|c| c.severity), expect);
    }
}
