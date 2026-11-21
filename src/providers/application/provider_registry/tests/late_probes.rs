//! Background probe answers: slow-probe retry and cache refresh.

use std::sync::Mutex;

use super::*;
use crate::providers::application::provider_registry::{LateMerge, LateProbes};

    fn slow_start() -> Runtime {
        Runtime::for_tests(vec![availability(Claude, State::Ready), availability(Codex, State::Unknown), availability(Gemini, State::NotLoggedIn)])
    }

    #[tokio::test]
    async fn a_provider_that_timed_out_at_startup_becomes_usable_once_its_retry_succeeds() {
        let mut rt = slow_start();
        assert!(!rt.usable(NOW).contains(some_model(Codex)), "unknown is not routed to");
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        // The fake CLI answers after 30 ms, well inside the retry cap.
        let probe = |p: ProviderId| async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            availability(p, State::Ready)
        };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, move |a| said_in.lock().unwrap().push(a.provider));
        assert!(!rt.absorb_late(), "nothing has arrived yet, and asking must not block");
        assert!(said.lock().unwrap().is_empty(), "no notice before the answer is applied");
        while rt.late.is_some() {
            rt.absorb_late();
            tokio::task::yield_now().await;
        }
        assert_eq!(*said.lock().unwrap(), vec![Codex], "announced once, when it was applied");
        assert_eq!(rt.avail.iter().find(|a| a.provider == Codex).map(|a| a.state), Some(State::Ready));
        assert!(rt.usable(NOW).contains(some_model(Codex)));
        assert_eq!(rt.avail.iter().find(|a| a.provider == Gemini).map(|a| a.state), Some(State::NotLoggedIn));
        assert!(!rt.absorb_late(), "applied once");
    }

    #[tokio::test]
    async fn a_background_refresh_corrects_a_cached_answer_and_announces_only_what_became_ready() {
        let mut rt = Runtime::for_tests(vec![availability(Claude, State::Ready), availability(Codex, State::NotLoggedIn), availability(Gemini, State::Ready)]);
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        rt.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(move |a| said_in.lock().unwrap().push(a.provider)) });
        let _ = tx.send(availability(Claude, State::Ready)); // same as cached: nothing to do
        let _ = tx.send(availability(Codex, State::Ready)); // logged in since the cache was written
        let _ = tx.send(availability(Gemini, State::NotLoggedIn)); // logged out since
        let _ = tx.send(availability(ProviderId::Grok, State::Unknown)); // could not tell: ignored
        assert!(rt.absorb_late());
        assert_eq!(*said.lock().unwrap(), vec![Codex], "only a provider that became ready is announced");
        let state = |rt: &Runtime, p| rt.avail.iter().find(|a| a.provider == p).map(|a| a.state);
        assert_eq!((state(&rt, Codex), state(&rt, Gemini)), (Some(State::Ready), Some(State::NotLoggedIn)));
        drop(tx);
        assert!(!rt.absorb_late());
        assert!(rt.late.is_none(), "the refresh ends with its channel");
    }

    #[tokio::test]
    async fn the_slow_probe_hook_attaches_to_a_running_refresh_instead_of_replacing_it() {
        let mut rt = Runtime::for_tests(vec![availability(Claude, State::Ready)]);
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        rt.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(|_| {}) });
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), |p| async move { availability(p, State::Ready) }, move |a| said_in.lock().unwrap().push(a.provider));
        let _ = tx.send(availability(Codex, State::Ready));
        rt.avail.push(availability(Codex, State::NotLoggedIn));
        rt.absorb_late();
        assert_eq!(*said.lock().unwrap(), vec![Codex]);
    }

    #[tokio::test]
    async fn a_retry_that_ends_unknown_again_changes_nothing_and_is_not_repeated() {
        let mut rt = slow_start();
        let probe = |p: ProviderId| async move { availability(p, State::Unknown) };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, |_| panic!("not ready"));
        while rt.late.is_some() {
            assert!(!rt.absorb_late());
            tokio::task::yield_now().await;
        }
        assert_eq!(rt.avail[1].state, State::Unknown);
        assert!(rt.late.is_none(), "one retry only: the receiver is gone");
    }

    #[tokio::test]
    async fn nothing_is_retried_when_every_probe_gave_an_answer() {
        let mut rt = runtime();
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), |p| async move { availability(p, State::Ready) }, |_| panic!("no retry"));
        assert!(rt.late.is_none());
    }

    #[tokio::test]
    async fn a_refresh_that_already_found_the_provider_is_not_overwritten_by_the_late_answer() {
        let mut rt = slow_start();
        let said = std::sync::Arc::new(Mutex::new(Vec::new()));
        let said_in = said.clone();
        // The late probe answers Ready, but only after the test has let a refresh settle it.
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(Some(gate)));
        let probe = move |p: ProviderId| {
            let gate = gate.clone();
            async move {
                if let Some(g) = gate.lock().await.take() {
                    let _ = g.await;
                }
                availability(p, State::Ready)
            }
        };
        rt.retry_slow_probes_with(std::time::Duration::from_secs(5), probe, move |a| said_in.lock().unwrap().push(a.provider));
        rt.apply_probe(vec![availability(Claude, State::Ready), availability(Codex, State::NotLoggedIn)]);
        release.send(()).unwrap();
        while rt.late.is_some() {
            rt.absorb_late();
            tokio::task::yield_now().await;
        }
        assert_eq!(rt.avail[1].state, State::NotLoggedIn);
        assert!(!rt.usable(NOW).contains(some_model(Codex)), "the refused answer does not make it routable");
        assert!(said.lock().unwrap().is_empty(), "and nobody is told it is ready: {:?}", said.lock().unwrap());
    }
