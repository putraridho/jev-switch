use super::*;
use ProviderId::{Codex, Gemini};

const NOW: u64 = 1_760_000_000_000;
const P: &str = "/work/proj#trusted";

fn key(cli: &str, account: &str) -> ReadinessKey {
    ReadinessKey { cli: cli.into(), backend: "Google login".into(), account: account.into() }
}

fn ready(p: ProviderId) -> Availability {
    Availability { provider: p, state: State::Ready, plan: None, models: None, detail: "agy login".into(), verified: true }
}

#[test]
fn each_failure_class_names_its_rung_and_policy() {
    let c = |k: ErrorKind| classify(&k);
    assert_eq!(c(ErrorKind::Auth).map(|c| (c.rung, c.policy)), Some((Rung::Authenticated, RetryPolicy::UntilChanged)));
    assert_eq!(c(ErrorKind::NotEntitled).map(|c| (c.rung, c.policy, c.scope)), Some((Rung::Entitled, RetryPolicy::UntilChanged, Scope::Model)));
    assert_eq!(c(ErrorKind::Unsupported).map(|c| (c.rung, c.policy)), Some((Rung::ProtocolCompatible, RetryPolicy::UntilChanged)));
    assert_eq!(c(ErrorKind::UnusableHere).map(|c| (c.rung, c.policy)), Some((Rung::ValidatedForProject, RetryPolicy::UntilChanged)));
    assert_eq!(c(ErrorKind::Empty).map(|c| c.policy), Some(RetryPolicy::Breaker));
    assert_eq!(c(ErrorKind::Stalled).map(|c| c.policy), Some(RetryPolicy::Breaker));
    assert!(c(ErrorKind::Quota { resets_at: None }).is_none(), "a limit has its own cooldown");
    assert!(c(ErrorKind::Other).is_none());
}

#[test]
fn a_lasting_failure_holds_until_it_expires_and_a_transient_one_is_not_stored() {
    let mut r = Readiness::default();
    let k = key("agy:1", "a");
    let auth = classify(&ErrorKind::Auth).unwrap();
    assert!(r.fail(Gemini, P, &k, auth, "login expired", NOW));
    assert_eq!(r.holding(Gemini, P, &k, NOW + 1).map(|f| f.rung), Some(Rung::Authenticated));
    assert!(r.holding(Gemini, P, &k, NOW + HOLD_MS).is_none(), "the hold expires");
    assert!(!r.fail(Gemini, P, &k, classify(&ErrorKind::Empty).unwrap(), "empty", NOW), "the breaker owns transient failures");
    assert!(!r.fail(Gemini, P, &k, classify(&ErrorKind::NotEntitled).unwrap(), "model", NOW), "a model refusal is not a provider hold");
}

#[test]
fn a_changed_cli_account_backend_or_project_voids_the_hold() {
    let mut r = Readiness::default();
    let k = key("agy:1", "a");
    r.fail(Gemini, P, &k, classify(&ErrorKind::Unsupported).unwrap(), "retired client", NOW);
    assert!(r.holding(Gemini, P, &k, NOW + 1).is_some());
    for changed in [key("agy:2", "a"), key("agy:1", "b"), ReadinessKey { backend: "API key (per token)".into(), ..k.clone() }] {
        assert!(r.holding(Gemini, P, &changed, NOW + 1).is_none(), "{changed:?}");
    }
    assert!(r.holding(Gemini, "/work/proj#untrusted", &k, NOW + 1).is_none(), "another project key is another record");
    assert!(r.holding(Codex, P, &k, NOW + 1).is_none());
}

#[test]
fn a_turn_that_answered_proves_the_ladder_and_ends_the_hold() {
    let mut r = Readiness::default();
    let k = key("c", "a");
    r.fail(Gemini, P, &k, classify(&ErrorKind::UnusableHere).unwrap(), "untrusted", NOW);
    assert!(r.verify_through(Gemini, P, &k, Rung::ValidatedForProject, "answered a turn", NOW + 5));
    assert!(r.holding(Gemini, P, &k, NOW + 6).is_none());
    assert_eq!(r.record(Gemini, P, &k).unwrap().evidence.len(), 5);
    assert!(!r.verify_through(Gemini, P, &k, Rung::ValidatedForProject, "answered a turn", NOW + 6), "fresh evidence is not rewritten");
}

#[test]
fn the_ladder_leaves_unknown_rungs_unverified_and_stops_at_a_failure() {
    let l = Ladder::build(&ready(Gemini), NOW, None, NOW);
    assert_eq!(l.reached(), Some(Rung::Authenticated), "entitled has no evidence, so nothing above counts");
    assert!(matches!(l.rows[2].1, RungState::Unverified));
    assert!(l.ready_for(Rung::Authenticated) && !l.ready_for(Rung::ProtocolCompatible));

    let mut r = Readiness::default();
    let k = key("c", "a");
    r.fail(Gemini, P, &k, classify(&ErrorKind::Unsupported).unwrap(), "retired client", NOW);
    let l = Ladder::build(&ready(Gemini), NOW, r.record(Gemini, P, &k), NOW + 1);
    assert_eq!(l.failure().map(|(rung, f)| (rung, f.what.as_str())), Some((Rung::ProtocolCompatible, "retired client")));
    assert!(!l.ready_for(Rung::Authenticated));
    let later = Ladder::build(&ready(Gemini), NOW, r.record(Gemini, P, &k), NOW + HOLD_MS);
    assert!(later.failure().is_none(), "an expired hold no longer fails the rung");
}

#[test]
fn a_login_the_probe_cannot_see_is_a_failed_first_rung() {
    let a = Availability { state: State::NotLoggedIn, detail: "run agy".into(), ..ready(Gemini) };
    let l = Ladder::build(&a, NOW, None, NOW);
    assert_eq!(l.failure().map(|(r, _)| r), Some(Rung::Authenticated));
    assert_eq!(l.reached(), Some(Rung::Installed));
}

#[test]
fn merging_keeps_the_newer_record_and_a_login_forgiveness_stays() {
    let (mut a, mut b) = (Readiness::default(), Readiness::default());
    let k = key("c", "a");
    a.fail(Gemini, P, &k, classify(&ErrorKind::Auth).unwrap(), "x", NOW);
    let mut saved = a.clone();
    assert!(a.forgive(Gemini, Rung::Authenticated, NOW + 10));
    assert!(a.merged(&saved).holding(Gemini, P, &k, NOW + 11).is_none());
    b.fail(Codex, P, &k, classify(&ErrorKind::Auth).unwrap(), "y", NOW + 1);
    saved = saved.merged(&b);
    assert_eq!(saved.entries().len(), 2);
}

#[test]
fn fixes_say_how_to_get_back_up() {
    assert!(fix_steps(Gemini, Rung::Authenticated).join(" ").contains("Log in"));
    assert!(fix_steps(Gemini, Rung::ProtocolCompatible).join(" ").contains("Update the CLI"));
    assert!(fix_steps(Gemini, Rung::ValidatedForProject).join(" ").contains("Trust this folder"));
    assert!(fix_steps(Codex, Rung::ValidatedForProject).join(" ").contains("git"));
}

#[test]
fn old_records_cap_at_a_bounded_number() {
    let mut r = Readiness::default();
    let k = key("c", "a");
    for i in 0..(MAX_RECORDS + 5) {
        r.fail(Gemini, &format!("/p{i}"), &k, classify(&ErrorKind::Auth).unwrap(), "x", NOW + i as u64);
    }
    assert_eq!(r.entries().len(), MAX_RECORDS);
}
