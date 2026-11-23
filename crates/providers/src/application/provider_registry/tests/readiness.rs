//! The readiness ladder on the runtime: lasting failures hold a provider until a fact changes.

use std::collections::HashSet;

use super::*;
use crate::domain::handshake::{Check, Handshake};
use crate::domain::readiness::{ReadinessKey, Rung, HOLD_MS};
use jev_backend::{ErrorKind, TurnError};

fn ready(p: ProviderId) -> Availability {
    Availability { provider: p, state: State::Ready, plan: None, models: None, detail: "agy login".into(), verified: false }
}

fn fail(rt: &mut Runtime, p: ProviderId, kind: ErrorKind, now: u64) -> Option<String> {
    rt.react(p, "m", &TurnError::new(kind, "x"), &mut HashSet::new(), now)
}

fn key(cli: &str) -> ReadinessKey {
    ReadinessKey { cli: cli.into(), backend: "Google login".into(), account: "t:1".into() }
}

fn runtime() -> Runtime {
    let mut rt = Runtime::for_tests(vec![ready(Gemini), ready(Claude)]);
    rt.keys.insert(Gemini, key("agy:1"));
    rt.project_keys.insert(Gemini, "/p#trusted".into());
    rt
}

#[test]
fn each_lasting_failure_holds_the_provider_at_its_rung() {
    for (kind, rung) in [
        (ErrorKind::Auth, Rung::Authenticated),
        (ErrorKind::Unsupported, Rung::ProtocolCompatible),
        (ErrorKind::UnusableHere, Rung::ValidatedForProject),
    ] {
        let mut rt = runtime();
        fail(&mut rt, Gemini, kind.clone(), NOW);
        assert!(!rt.usable(NOW + 1).has_provider(Gemini), "{kind:?}");
        assert!(rt.usable(NOW + 1).has_provider(Claude));
        let why = rt.exclusion(Gemini, NOW + 1).unwrap();
        assert!(why.contains(&format!("held at the {} rung", rung.label())), "{why}");
        assert_eq!(rt.ladder(Gemini, NOW + 1).unwrap().failure().map(|f| f.0), Some(rung));
        assert!(rt.paused_left(Gemini, NOW + 1).is_none(), "the breaker is for transient failures");
    }
}

#[test]
fn the_hold_outlives_the_session_but_not_a_changed_fact_or_its_expiry() {
    let dir = jev_testkit::scratch_dir::scratch_dir("readiness-runtime", "next-run");
    let path = dir.join("provider-readiness.json");
    let mut first = runtime();
    first.readiness_path = Some(path.clone());
    fail(&mut first, Gemini, ErrorKind::Unsupported, NOW);
    assert!(path.exists());

    let mut second = runtime();
    second.readiness = second.host.load_readiness(&path);
    assert!(!second.usable(NOW + 1).has_provider(Gemini), "a new session does not rediscover it");
    assert!(second.usable(NOW + HOLD_MS).has_provider(Gemini), "the hold expires");

    second.keys.insert(Gemini, key("agy:2"));
    assert!(second.usable(NOW + 1).has_provider(Gemini), "an updated CLI voids the verdict");
    second.keys.insert(Gemini, ReadinessKey { account: "t:2".into(), ..key("agy:1") });
    assert!(second.usable(NOW + 1).has_provider(Gemini), "a new login voids it");
    second.keys.insert(Gemini, ReadinessKey { backend: "API key (per token)".into(), ..key("agy:1") });
    assert!(second.usable(NOW + 1).has_provider(Gemini), "another backend voids it");
    second.keys.insert(Gemini, key("agy:1"));
    second.project_keys.insert(Gemini, "/p#untrusted".into());
    assert!(second.usable(NOW + 1).has_provider(Gemini), "a changed folder trust voids it");
}

#[test]
fn an_answer_or_a_refresh_ends_the_right_holds() {
    let mut rt = runtime();
    fail(&mut rt, Gemini, ErrorKind::UnusableHere, NOW);
    rt.blocked.clear(); // the session block is separate from the saved hold
    rt.note_success(Gemini, NOW + 5);
    assert!(rt.usable(NOW + 6).has_provider(Gemini), "an answered turn proves the folder works");
    assert_eq!(rt.ladder(Gemini, NOW + 6).unwrap().reached(), Some(Rung::ValidatedForProject));

    rt.blocked.clear();
    fail(&mut rt, Gemini, ErrorKind::Auth, NOW + 10);
    rt.blocked.clear();
    rt.forgive_logins(NOW + 11);
    assert!(rt.usable(NOW + 12).has_provider(Gemini), "a refresh that sees the login forgives a login failure");
    fail(&mut rt, Gemini, ErrorKind::Unsupported, NOW + 20);
    rt.blocked.clear();
    rt.forgive_logins(NOW + 21);
    assert!(!rt.usable(NOW + 22).has_provider(Gemini), "a refresh does not forgive a retired client");
}

#[test]
fn a_model_the_plan_lacks_is_remembered_for_the_project_not_the_provider() {
    let mut rt = runtime();
    fail(&mut rt, Gemini, ErrorKind::NotEntitled, NOW);
    assert!(rt.usable(NOW + 1).has_provider(Gemini));
    let (project, k) = rt.scope(Gemini);
    assert_eq!(rt.readiness.record(Gemini, &project, &k).unwrap().refused, vec!["m".to_string()]);
}

#[test]
fn the_handshake_proves_rungs_and_a_failed_check_holds() {
    let mut rt = runtime();
    rt.apply_handshake(Gemini, &Handshake { protocol: Check::Passed("answered ACP initialize".into()), project: Check::Unverified("no trust decision".into()) }, NOW);
    let ladder = rt.ladder(Gemini, NOW + 1).unwrap();
    assert_eq!(ladder.reached(), Some(Rung::Authenticated), "entitled is still unverified, so the climb stops there");
    assert!(ladder.rows[3].1 != crate::domain::readiness::RungState::Unverified);
    assert!(rt.usable(NOW + 1).has_provider(Gemini));

    rt.apply_handshake(Gemini, &Handshake { protocol: Check::Passed("ok".into()), project: Check::Failed("gemini refuses an untrusted folder".into()) }, NOW + 2);
    assert!(!rt.usable(NOW + 3).has_provider(Gemini));
    let text = rt.render(NOW + 3);
    assert!(text.contains("readiness: gemini: ") && text.contains("validated for this project failed: gemini refuses an untrusted folder"), "{text}");
    let view = rt.readiness_views(NOW + 3).into_iter().find(|v| v.provider == Gemini).unwrap();
    assert!(view.fix.join(" ").contains("Trust this folder"));
}

#[test]
fn providers_below_the_needed_rung_are_excluded_with_a_reason() {
    let rt = Runtime::for_tests(vec![Availability { state: State::NotLoggedIn, detail: "run agy".into(), ..ready(Gemini) }]);
    assert!(!rt.usable(NOW).has_provider(Gemini));
    assert!(rt.exclusion(Gemini, NOW).unwrap().contains("below the logged in rung"));
    assert!(runtime().exclusion(Claude, NOW).is_none());
}

#[test]
fn a_new_session_ends_a_folder_hold_after_the_folder_was_fixed() {
    let mut rt = runtime();
    fail(&mut rt, Gemini, ErrorKind::UnusableHere, NOW);
    rt.forget_unusable_here();
    assert!(rt.usable(NOW + 1).has_provider(Gemini));
}
