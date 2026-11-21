//! The time cap, the slow-probe retry and the merging of late answers.

use std::time::Duration;

use super::ready;
use crate::providers::domain::availability::retry::{merge_late, merge_refresh, retry_candidates, retry_unknown, RETRY_CAP};
use crate::providers::domain::availability::{avail, capped, Availability, State};
use crate::backend::ProviderId;

#[tokio::test]
async fn slow_probe_becomes_unknown() {
    let never = std::future::pending::<Availability>();
    let a = capped(Duration::from_millis(30), ProviderId::Grok, never).await;
    assert_eq!((a.provider, a.state), (ProviderId::Grok, State::Unknown));
    assert!(a.detail.contains("timed out") && !a.verified);
}

#[tokio::test]
async fn fast_probe_passes_through() {
    let a = capped(Duration::from_secs(1), ProviderId::Claude, async { ready(ProviderId::Claude, None) }).await;
    assert_eq!(a.state, State::Ready);
}

// ---- the slow-probe retry

fn unknown(p: ProviderId) -> Availability {
    avail(p, State::Unknown, "probe timed out after 5s")
}

#[test]
fn only_providers_that_could_not_tell_are_retried() {
    let first = [ready(ProviderId::Claude, None), unknown(ProviderId::Codex), avail(ProviderId::Gemini, State::NotLoggedIn, "x"), unknown(ProviderId::Grok)];
    assert_eq!(retry_candidates(&first), [ProviderId::Codex, ProviderId::Grok]);
    assert!(retry_candidates(&[ready(ProviderId::Claude, None)]).is_empty());
}

#[tokio::test]
async fn the_retry_asks_each_provider_once_under_the_cap_and_reports_answers_as_they_come() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    // codex answers after 20 ms; claude never answers; both under a 150 ms cap.
    let fake = move |p: ProviderId| {
        counter.fetch_add(1, Ordering::SeqCst);
        async move {
            if p == ProviderId::Codex {
                tokio::time::sleep(Duration::from_millis(20)).await;
                ready(p, None)
            } else {
                std::future::pending().await
            }
        }
    };
    let mut answers = Vec::new();
    retry_unknown(vec![ProviderId::Claude, ProviderId::Codex], Duration::from_millis(150), fake, |a| answers.push((a.provider, a.state))).await;
    assert_eq!(answers, [(ProviderId::Codex, State::Ready), (ProviderId::Claude, State::Unknown)], "the quick one is not held back by the slow one");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "one retry each, never a loop");
}

#[tokio::test]
async fn a_retry_with_nothing_to_retry_does_nothing() {
    let mut seen = 0;
    retry_unknown(Vec::new(), RETRY_CAP, |p| async move { ready(p, None) }, |_| seen += 1).await;
    assert_eq!(seen, 0);
}

#[test]
fn a_fresh_answer_replaces_a_cached_one_only_when_it_differs_and_is_definite() {
    let mut cached = vec![ready(ProviderId::Claude, None), avail(ProviderId::Codex, State::NotLoggedIn, "x")];
    assert!(!merge_refresh(&mut cached, ready(ProviderId::Claude, None)), "same answer");
    assert!(!merge_refresh(&mut cached, avail(ProviderId::Claude, State::Unknown, "slow")), "unknown teaches nothing");
    assert!(merge_refresh(&mut cached, avail(ProviderId::Claude, State::NotLoggedIn, "logged out")));
    assert!(merge_refresh(&mut cached, ready(ProviderId::Codex, None)));
    assert_eq!((cached[0].state, cached[1].state), (State::NotLoggedIn, State::Ready));
}

#[test]
fn a_late_answer_fills_in_only_a_provider_that_is_still_unknown() {
    let mut now = vec![ready(ProviderId::Claude, None), unknown(ProviderId::Codex), avail(ProviderId::Gemini, State::NotLoggedIn, "x")];
    assert!(merge_late(&mut now, ready(ProviderId::Codex, Some(vec!["m"]))));
    assert_eq!((now[1].state, now[1].models.clone()), (State::Ready, Some(vec!["m".to_string()])));
    assert!(!merge_late(&mut now, avail(ProviderId::Codex, State::NotLoggedIn, "x")), "a definite answer is never overwritten");
    assert!(!merge_late(&mut now, ready(ProviderId::Gemini, None)), "nor is a refresh's result");
    assert_eq!(now[2].state, State::NotLoggedIn);
    let mut still = vec![unknown(ProviderId::Codex)];
    assert!(!merge_late(&mut still, unknown(ProviderId::Codex)), "Unknown teaches nothing");
}
