use super::*;
use startup_hint::first_run_hint;
use std::cmp::Reverse;

#[test]
fn routing_note_by_ready_count() {
    use State::*;
    let none = diagnose(&inputs(all([NotInstalled; 5])));
    assert!(none.routing_note.starts_with("No provider is ready"));
    let one = diagnose(&inputs(all([Ready, NoKey, NotInstalled, NotInstalled, NotInstalled])));
    assert_eq!(one.routing_note, "Only claude is ready, so every turn runs on claude. Add a second provider to route by task type.");
    let two = diagnose(&inputs(all([Ready, NoKey, Ready, NotInstalled, NotInstalled])));
    assert_eq!(two.routing_note, "Routing across claude, codex.");
    assert_eq!(two.ready_providers, vec![Claude, Codex]);
}

#[test]
fn a_blocked_provider_is_not_counted_ready() {
    let mut i = inputs(all([State::Ready, State::NoKey, State::Ready, State::NotInstalled, State::NotInstalled]));
    i.blocked = vec![(Codex, "its login failed".into())];
    let r = diagnose(&i);
    assert_eq!(r.ready_providers, vec![Claude]);
    assert!(find(&r, "blocked").detail.contains("codex: its login failed"));
}

#[test]
fn no_ready_provider_is_a_problem() {
    let r = diagnose(&inputs(all([State::NotInstalled; 5])));
    assert_eq!(find(&r, "no-provider").severity, Severity::Problem);
    let r = diagnose(&inputs(all([State::Ready, State::NoKey, State::NotInstalled, State::NotInstalled, State::NotInstalled])));
    assert!(!has_check(&r, "no-provider"));
}

#[test]
fn checks_are_ordered_problem_warn_info_ok() {
    use State::*;
    let mut i = inputs(all([Ready, NoKey, NotLoggedIn, NotInstalled, Ready]));
    i.jev_key_present = false;
    i.jev_reachable = None;
    i.in_git_repo = Some(false);
    i.cooldown_lines = vec!["claude cooling for 1m".into()];
    let r = diagnose(&i);
    let sev: Vec<Severity> = r.checks.iter().map(|c| c.severity).collect();
    let mut sorted = sev.clone();
    sorted.sort_by_key(|s| Reverse(*s));
    assert_eq!(sev, sorted);
    assert_eq!(r.checks[0].id, "jev-key");
    assert_eq!(r.checks.last().unwrap().severity, Severity::Ok);
    assert!(sev.contains(&Severity::Problem) && sev.contains(&Severity::Warn) && sev.contains(&Severity::Info));
}

#[test]
fn empty_inputs_never_panic() {
    let r = diagnose(&Inputs::default());
    assert!(r.ready_providers.is_empty());
    assert!(!r.routing_note.is_empty());
    assert!(has_check(&r, "jev-key") && has_check(&r, "no-provider"));
    assert!(first_run_hint(&[]).is_some());
}
