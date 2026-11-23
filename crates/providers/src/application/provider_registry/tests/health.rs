//! Circuit breakers on the runtime: what counts, what pauses, what forgives.

use std::collections::HashSet;

use super::*;
use crate::domain::breaker::{FIRST_PAUSE_MS, TRIP_AFTER};
use jev_backend::{ErrorKind, TurnError};

fn ready(p: ProviderId) -> Availability {
    Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true }
}

fn fail(rt: &mut Runtime, p: ProviderId, kind: ErrorKind, times: u32, now: u64) {
    for _ in 0..times {
        rt.react(p, "m", &TurnError::new(kind.clone(), "x"), &mut HashSet::new(), now);
    }
}

#[test]
fn a_provider_that_keeps_giving_empty_answers_is_paused_then_comes_back() {
    let mut rt = Runtime::for_tests(vec![ready(Claude), ready(Gemini)]);
    fail(&mut rt, Gemini, ErrorKind::Empty, TRIP_AFTER - 1, NOW);
    assert!(rt.usable(NOW).has_provider(Gemini) && rt.paused_left(Gemini, NOW).is_none());
    fail(&mut rt, Gemini, ErrorKind::Empty, 1, NOW);
    assert!(!rt.usable(NOW).has_provider(Gemini), "paused");
    assert!(rt.usable(NOW).has_provider(Claude), "others are untouched");
    assert_eq!(rt.paused_left(Gemini, NOW), Some(FIRST_PAUSE_MS));
    let text = rt.render(NOW);
    assert!(text.contains("gemini paused for") && text.contains("last: empty answer") && text.contains("/providers refresh"), "{text}");
    assert!(rt.usable(NOW + FIRST_PAUSE_MS).has_provider(Gemini), "the pause ends by itself");
}

#[test]
fn only_failures_that_blame_the_provider_count() {
    let mut rt = Runtime::for_tests(vec![ready(Gemini)]);
    for kind in [ErrorKind::Quota { resets_at: None }, ErrorKind::NotEntitled, ErrorKind::Auth, ErrorKind::UnusableHere, ErrorKind::Unsupported, ErrorKind::Other] {
        fail(&mut rt, Gemini, kind, TRIP_AFTER * 2, NOW);
    }
    assert!(rt.paused_left(Gemini, NOW).is_none());
    fail(&mut rt, Gemini, ErrorKind::Stalled, TRIP_AFTER, NOW);
    assert!(rt.paused_left(Gemini, NOW).is_some());
}

#[test]
fn an_answer_forgives_and_a_refresh_forgets() {
    let mut rt = Runtime::for_tests(vec![ready(Gemini), ready(Codex)]);
    fail(&mut rt, Gemini, ErrorKind::Empty, TRIP_AFTER, NOW);
    rt.note_success(Gemini, NOW + 1);
    assert!(rt.paused_left(Gemini, NOW + 1).is_none());
    fail(&mut rt, Gemini, ErrorKind::Empty, TRIP_AFTER - 1, NOW + 2);
    assert!(rt.paused_left(Gemini, NOW + 2).is_none(), "the count started over");

    fail(&mut rt, Codex, ErrorKind::Stalled, TRIP_AFTER, NOW);
    assert!(rt.paused_left(Codex, NOW).is_some());
    rt.forget_pauses(NOW + 3);
    assert!(rt.paused_left(Codex, NOW + 3).is_none());
}

#[test]
fn a_pause_is_remembered_by_the_next_run() {
    let dir = jev_testkit::scratch_dir::scratch_dir("health-runtime", "next-run");
    let path = dir.join("provider-health.json");
    let mut first = Runtime::for_tests(vec![ready(Gemini)]);
    first.breaker_path = Some(path.clone());
    fail(&mut first, Gemini, ErrorKind::Empty, TRIP_AFTER, NOW);
    assert!(path.exists());

    let mut second = Runtime::for_tests(vec![ready(Gemini)]);
    second.breaker_path = Some(path.clone());
    second.breakers = second.host.load_breakers(&path);
    assert!(!second.usable(NOW + 1).has_provider(Gemini));

    second.note_success(Gemini, NOW + 2);
    let mut third = Runtime::for_tests(vec![ready(Gemini)]);
    third.breakers = third.host.load_breakers(&path);
    assert!(third.usable(NOW + 3).has_provider(Gemini), "the forgiveness was saved too");
}

#[test]
fn after_the_pause_one_trial_attempt_decides() {
    let mut rt = Runtime::for_tests(vec![ready(Gemini), ready(Claude)]);
    fail(&mut rt, Gemini, ErrorKind::Empty, TRIP_AFTER, NOW);
    assert!(!rt.usable(NOW).has_provider(Gemini));
    let later = NOW + FIRST_PAUSE_MS;
    assert!(rt.usable(later).has_provider(Gemini), "half-open: one attempt may go");
    rt.begin_attempt(Gemini, later);
    assert!(!rt.usable(later + 1).has_provider(Gemini), "others wait for the trial");
    assert!(rt.render(later + 1).contains("being tried after a pause"));
    // The trial fails: the breaker opens again, for longer.
    fail(&mut rt, Gemini, ErrorKind::Empty, 1, later + 2);
    assert_eq!(rt.paused_left(Gemini, later + 2), Some(2 * FIRST_PAUSE_MS));
    // The next trial answers: closed.
    let again = later + 2 + 2 * FIRST_PAUSE_MS;
    rt.begin_attempt(Gemini, again);
    rt.note_success(Gemini, again + 1);
    assert!(rt.usable(again + 2).has_provider(Gemini) && rt.paused_left(Gemini, again + 2).is_none());
}
