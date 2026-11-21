//! Stopping a turn that is running, between attempts, or being judged.

use super::*;

#[tokio::test]
async fn a_cancel_token_is_shared_by_its_clones_and_resolves_once_cancelled() {
    let c = Cancel::new();
    let other = c.clone();
    assert!(!c.is_cancelled());
    assert!(tokio::time::timeout(Duration::from_millis(50), c.cancelled()).await.is_err(), "pending until cancelled");
    other.cancel();
    other.cancel();
    assert!(c.is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), c.cancelled()).await.expect("resolves at once when already cancelled");
    assert!(!Cancel::default().is_cancelled());
}

#[tokio::test]
async fn cancelling_a_running_turn_returns_quickly_without_retry_or_an_answer() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, HANGS);
    let cancel = Cancel::new();
    r.ui.cancel_on_tool = Some(cancel.clone());
    let log = std::env::temp_dir().join(format!("jev-cancel-log-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    r.engine.turn_log = Some(log.to_string_lossy().into_owned());
    let routing_before = r.engine.routing.current.clone();

    let started = Instant::now();
    let report = r
        .engine
        .run_judged_cancellable("add a retry to the client", Judged::Providers(Some(want(TaskType::Feature))), &mut r.ui, &cancel)
        .await;
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(report.status, TurnStatus::Cancelled);
    assert_eq!((report.attempts, report.provider), (1, Some(a)));
    assert!(report.text.is_empty() && report.usage.is_none());
    assert!(report.message.contains("cancelled"));

    // Nothing was retried on the other provider, and nothing else moved.
    assert_eq!(r.started().len(), 1);
    assert_eq!(r.engine.routing.current, routing_before);
    assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
    assert!(r.engine.runtime.unusable_models.is_empty() && r.engine.runtime.blocked.is_empty());
    // Half an answer was never shown as one; the tool progress was.
    assert!(r.ui.texts.is_empty(), "{:?}", r.ui.texts);
    assert!(r.ui.seen.contains(&"tool:Read a.rs".to_string()));
    assert!(r.engine.sessions.session_id(a).is_none(), "a cancelled turn gives no session id");

    // The next catch-up tells the model the turn was cut short.
    let b = if a == Claude { Codex } else { Claude };
    let text = r.engine.sessions.catch_up(b, "", 10_000).expect("recorded");
    assert!(text.contains("add a retry to the client") && text.contains("FAILED"), "{text}");
    assert!(text.contains("interrupted by the user before it finished; files may be partly changed"), "{text}");
    assert!(!text.contains("half an answer"));
    // The turn log row says so.
    let row: Value = serde_json::from_str(std::fs::read_to_string(&log).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!((row["ok"].clone(), row["error_kind"].clone(), row["cancelled"].clone()), (json!(false), json!("cancelled"), json!(true)));
    let _ = std::fs::remove_file(&log);

    // The session goes on: the same model takes the next turn.
    r.ui.cancel_on_tool = None;
    r.script(a, OK);
    let next = r.turn("try again", want(TaskType::Feature)).await;
    assert_eq!((next.status, next.provider), (TurnStatus::Answered, Some(a)));
    assert!(r.started()[1].prompt.contains("try again"));
}

#[tokio::test]
async fn the_provider_that_ran_a_cancelled_turn_is_told_about_it_on_its_next_prompt() {
    let mut r = rig(&[Codex]);
    r.script(Codex, HANGS);
    let cancel = Cancel::new();
    r.ui.cancel_on_tool = Some(cancel.clone());
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    let report = r.engine.run_judged_cancellable("rename the client", judged, &mut r.ui, &cancel).await;
    assert_eq!((report.status, report.provider), (TurnStatus::Cancelled, Some(Codex)));
    assert_eq!(r.engine.sessions.missed_turns(Codex), 1, "its own CLI session never recorded the killed turn");

    r.ui.cancel_on_tool = None;
    r.script(Codex, OK);
    let next = r.turn("try again", want(TaskType::Feature)).await;
    assert_eq!((next.status, next.provider), (TurnStatus::Answered, Some(Codex)));
    let prompt = &r.started()[1].prompt;
    assert!(prompt.contains("rename the client") && prompt.contains("interrupted by the user"), "{prompt}");
    assert!(asked(prompt).ends_with("try again"), "{prompt}");
    assert_eq!(r.engine.sessions.missed_turns(Codex), 0, "told once, then current");
}

#[tokio::test]
async fn a_cancel_between_attempts_keeps_the_real_failure_and_adds_the_interruption() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, QUOTA);
    let cancel = Cancel::new();
    // The failover note is shown after the first attempt failed, before the second starts.
    r.ui.cancel_on_note = Some(cancel.clone());
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    let report = r.engine.run_judged_cancellable("add a retry", judged, &mut r.ui, &cancel).await;
    assert_eq!((report.status, report.attempts), (TurnStatus::Cancelled, 1));
    assert_eq!(r.started().len(), 1, "the second model was never started");

    assert_eq!(r.engine.sessions.turns(), 2, "the failure and the interruption are two records");
    let b = if a == Claude { Codex } else { Claude };
    let text = r.engine.sessions.catch_up(b, "", 10_000).unwrap();
    assert!(text.contains("usage limit reached"), "the real error is kept: {text}");
    assert!(text.contains("interrupted by the user"), "{text}");
    assert!(text.find("usage limit reached") < text.find("interrupted by the user"), "failure first: {text}");
}

#[tokio::test]
async fn a_token_cancelled_before_anything_starts_starts_nothing_and_records_nothing() {
    let mut r = rig(&[Claude]);
    let cancel = Cancel::new();
    cancel.cancel();
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    let report = r.engine.run_judged_cancellable("hello there", judged, &mut r.ui, &cancel).await;
    assert_eq!((report.status, report.attempts), (TurnStatus::Cancelled, 0));
    assert!(r.factory_calls.lock().unwrap().is_empty());
    assert_eq!(r.engine.sessions.turns(), 0, "no model saw the prompt, so there is nothing to tell the next one");
}

#[tokio::test]
async fn cancelling_while_jev_is_being_asked_ends_the_turn_too() {
    let mut r = rig(&[Claude]);
    let cancel = Cancel::new();
    cancel.cancel();
    let report = r.engine.run_cancellable("hello there", &mut r.ui, &cancel).await;
    assert_eq!(report.status, TurnStatus::Cancelled);
    assert!(r.factory_calls.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_cancelled_turn_takes_the_cli_and_its_helpers_with_it() {
    let dir = std::env::temp_dir().join(format!("jev-cancel-kids-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("pid");
    let script: &'static str =
        Box::leak(format!("sleep 60 & echo $! > {}; echo 'TOOL:Read a.rs'; wait", pidfile.display()).into_boxed_str());
    let mut r = rig(&[Claude]);
    r.script(Claude, script);
    let cancel = Cancel::new();
    r.ui.cancel_on_tool = Some(cancel.clone());
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    let report = r.engine.run_judged_cancellable("go", judged, &mut r.ui, &cancel).await;
    assert_eq!(report.status, TurnStatus::Cancelled);
    let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    for _ in 0..200 {
        if !backend::process_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!backend::process_alive(pid), "the helper process outlived the cancelled turn");
    std::fs::remove_dir_all(&dir).ok();
}
