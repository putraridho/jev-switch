//! How a failed attempt moves on to the next provider, and what the UI sees of it.

use super::*;

#[tokio::test]
async fn a_turn_that_leaves_a_command_running_or_promises_to_report_back_says_to_ask_again() {
    let mut r = rig_with(&[(Codex, State::Ready)], "printf 'TOOL:Bash npm test &\\nFIN:Started the tests.\\n'");
    r.turn("run the tests", want(TaskType::Feature)).await;
    assert!(r.ui.seen.iter().any(|s| s.starts_with("hint:") && s.contains("1 command was left running")), "{:?}", r.ui.seen);

    let mut r = rig_with(&[(Codex, State::Ready)], "printf 'TOOL:Bash cargo test\\nFIN:Running them now. I will update you as soon as they finish.\\n'");
    r.turn("run the tests", want(TaskType::Feature)).await;
    assert!(r.ui.seen.iter().any(|s| s.starts_with("hint:") && s.contains("said it would report back")), "{:?}", r.ui.seen);

    let mut r = rig_with(&[(Codex, State::Ready)], OK);
    r.turn("run the tests", want(TaskType::Feature)).await;
    assert!(!r.ui.seen.iter().any(|s| s.starts_with("hint:")), "a turn that finished its work says nothing: {:?}", r.ui.seen);
}

#[tokio::test]
async fn quota_cools_the_provider_reruns_the_same_prompt_and_skips_it_next_turn() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    let b = if a == Claude { Codex } else { Claude };
    r.script(a, QUOTA);

    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.provider, Some(b));
    assert_eq!(report.text, "the answer");
    assert_eq!(report.attempts, 2);
    let s = r.started();
    assert_eq!(s.iter().map(|x| x.provider).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(asked(&s[1].prompt), "add a retry to the client", "the same prompt, nothing to catch up on yet");
    assert!(r.engine.runtime.cooldowns.active(a, NOW).is_some());
    assert!(r.ui.notes.iter().any(|n| n.contains("hit its limit") && n.contains("trying the next model")));
    // The failed attempt's output is never the answer.
    assert!(r.ui.texts.iter().all(|t| t == "the answer"));

    let again = r.turn("and add a test", want(TaskType::Feature)).await;
    assert_eq!(again.provider, Some(b));
    assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 1, "A is skipped next turn");
}

#[tokio::test]
async fn not_entitled_marks_the_model_unusable_and_never_retries_it() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, NOT_ENTITLED);

    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_ne!(report.provider, Some(a));
    let refused: Vec<_> = r.engine.runtime.unusable_models.iter().cloned().collect();
    assert!(refused.iter().all(|(p, _)| *p == a) && !refused.is_empty(), "{refused:?}");

    r.turn(LONG, new_task(TaskType::Feature)).await;
    for pair in &refused {
        assert_eq!(r.started().iter().filter(|x| (x.provider, x.model.clone()) == *pair).count(), 1, "{pair:?}");
    }
}

#[tokio::test]
async fn empty_and_stalled_exclude_for_this_turn_only() {
    for (script, expect) in [(EMPTY, "empty answer"), (STALL, "stopped answering")] {
        let ready = [Claude, Codex];
        let mut r = rig(&ready);
        let a = first_choice(TaskType::Feature, &ready);
        r.script(a, script);
        let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
        assert_eq!(report.status, TurnStatus::Answered, "{script}");
        assert_ne!(report.provider, Some(a));
        assert!(r.ui.notes.iter().any(|n| n.contains(expect) || n.contains("empty")), "{:?}", r.ui.notes);
        // Nothing lasting: no cooldown, no refused model, and A can serve the next turn.
        assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
        assert!(r.engine.runtime.unusable_models.is_empty());
        r.script(a, OK);
        let next = r.turn(LONG, new_task(TaskType::Feature)).await;
        assert_eq!(next.provider, Some(a), "{script}: A is usable again next turn");
    }
}

#[tokio::test]
async fn other_errors_stop_at_once_and_are_remembered_as_failed() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    let b = if a == Claude { Codex } else { Claude };
    r.script(a, OTHER);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Failed);
    assert_eq!(report.attempts, 1);
    assert!(report.text.is_empty());
    assert_eq!(r.started().len(), 1, "no retry on another provider");
    assert!(report.message.contains("segmentation fault"));
    let catch_up = r.engine.sessions.catch_up(b, "", 10_000).expect("the failed turn is on record");
    assert!(catch_up.contains("FAILED") && catch_up.contains("add a retry to the client"));
}

#[tokio::test]
async fn auth_failure_blocks_the_provider_for_the_session_and_names_the_tool_to_log_in_to() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, AUTH);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    // A login problem is not a usage limit: it is kept out of the persisted cooldowns,
    // blocked for this session, and cleared by `/providers refresh`.
    assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none());
    assert_eq!(r.engine.runtime.blocked.get(&a), Some(&crate::providers::application::provider_registry::BlockReason::Auth));
    assert!(r.ui.notes.iter().any(|n| n.contains("log in to") && n.contains(crate::providers::application::provider_registry::login_target(a))), "{:?}", r.ui.notes);
}

#[tokio::test]
async fn a_directory_refusal_fails_over_blocks_the_provider_and_new_session_lifts_it() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    let b = if a == Claude { Codex } else { Claude };
    r.script(a, UNUSABLE_HERE);

    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.provider, Some(b));
    assert_eq!(report.attempts, 2);
    assert_eq!(r.started().iter().map(|x| x.provider).collect::<Vec<_>>(), vec![a, b], "rerun on the other provider");
    assert_eq!(r.engine.runtime.blocked.get(&a), Some(&crate::providers::application::provider_registry::BlockReason::UnusableHere));
    assert!(r.engine.runtime.cooldowns.active(a, NOW).is_none(), "not a usage limit");
    assert!(r.ui.notes.iter().any(|n| n.contains("trying the next model") && n.contains(&a.to_string())), "{:?}", r.ui.notes);

    // Blocked for the rest of the session, even after a probe ...
    r.engine.runtime.avail = ready.iter().map(|p| availability(*p, State::Ready)).collect();
    r.turn(LONG, new_task(TaskType::Feature)).await;
    assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 1, "never started again");
    // ... until /new.
    r.engine.new_session();
    assert!(r.engine.runtime.blocked.is_empty());
    r.script(a, OK);
    r.script(b, UNUSABLE_HERE);
    r.turn(LONG, new_task(TaskType::Feature)).await;
    assert_eq!(r.started().iter().filter(|x| x.provider == a).count(), 2);
}

#[tokio::test]
async fn a_failed_attempts_text_never_reaches_the_ui_and_the_note_says_so() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, FAILS_AFTER_TALKING);

    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert!(r.ui.texts.iter().all(|t| t == "the answer"), "{:?}", r.ui.texts);
    assert!(r.ui.notes.iter().any(|n| n.contains("(output of the failed attempt discarded)")), "{:?}", r.ui.notes);
    // The failed attempt's tool call was progress and stays visible, ahead of the failover.
    let tool = r.ui.seen.iter().position(|s| s == "tool:Read a.rs").expect("tool shown");
    let note = r.ui.seen.iter().position(|s| s == "note:").expect("failover note");
    assert!(tool < note);
}

#[tokio::test]
async fn a_failed_attempt_without_text_has_nothing_to_say_was_discarded() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, QUOTA);
    r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert!(r.ui.notes.iter().all(|n| !n.contains("discarded")), "{:?}", r.ui.notes);
}

#[tokio::test]
async fn a_successful_attempts_text_arrives_in_order_after_its_tools_which_stream_live() {
    let mut r = rig(&[Claude]);
    r.script(Claude, TALKS_THEN_ANSWERS);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    // The tool came through while the attempt ran, ahead of text that was said before it.
    assert_eq!(
        &r.ui.seen[1..],
        ["tool:Edit b.rs", "text:first", "text:second", "text:final"],
        "{:?}",
        r.ui.seen
    );

    // An answer already among the texts is not shown a second time.
    r.script(Claude, ANSWER_ALREADY_SAID);
    r.ui = Recorder::default();
    r.turn("and add a test", want(TaskType::Feature)).await;
    assert_eq!(r.ui.texts, ["thinking", "done"]);

    r.script(Claude, "printf 'TEXT:first\\nTEXT:second\\nFIN:first second\\n'");
    r.ui = Recorder::default();
    r.turn("and explain", want(TaskType::Explain)).await;
    assert_eq!(r.ui.texts, ["first", "second"]);
}

#[tokio::test]
async fn at_most_six_models_are_tried() {
    let all = [Claude, DeepSeek, Codex, Gemini, Grok];
    let mut r = rig(&all);
    r.engine.runtime.allow = Allowances { credits: true, per_token: true };
    for p in all {
        r.script(p, EMPTY);
    }
    // A very hard debug turn has seven models in its chain: more than the cap.
    let hard = ProviderWant { very_hard_p: 0.95, ..want(TaskType::HardDebug) };
    let report = r.turn("find why the build hangs", hard).await;
    assert_eq!(report.status, TurnStatus::Failed);
    assert_eq!(report.attempts, MAX_ATTEMPTS);
    assert_eq!(r.started().len(), MAX_ATTEMPTS as usize);
    assert!(report.message.contains("stopped after 6 attempts"), "{}", report.message);
    assert!(report.text.is_empty());
}

#[tokio::test]
async fn the_answer_is_shown_once_even_when_the_cli_only_sends_it_at_the_end() {
    let mut r = rig(&[Claude]);
    r.turn("one", want(TaskType::Feature)).await;
    assert_eq!(r.ui.texts, vec!["the answer".to_string()]);
}

#[tokio::test]
async fn time_comes_from_the_injected_clock() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, QUOTA);
    r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert!(r.engine.runtime.cooldowns.active(a, NOW + 1).is_some());
    // Past the longest possible cooldown the provider is back, with no real waiting.
    r.clock.store(NOW + 9 * 24 * 3_600_000, Ordering::SeqCst);
    r.script(a, OK);
    let report = r.turn(LONG, new_task(TaskType::Feature)).await;
    assert_eq!(report.provider, Some(a));
}

#[tokio::test]
async fn a_duration_reset_counts_from_when_the_error_arrived_not_from_when_the_attempt_began() {
    const MINUTE: u64 = 60_000;
    const RESETS_IN_3H20: &str = "printf 'FAIL:You have hit your limit, resets in 3h20m\\n'";
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, RESETS_IN_3H20);
    r.attempt_ms.store(10 * MINUTE, Ordering::SeqCst); // the turn ran 10 minutes before failing

    r.turn("add a retry to the client", want(TaskType::Feature)).await;
    let reset = NOW + 10 * MINUTE + 200 * MINUTE;
    assert_eq!(r.engine.runtime.cooldowns.active(a, NOW), Some(reset - NOW));
    assert!(r.engine.runtime.cooldowns.active(a, reset - 1).is_some(), "not over 10 minutes early");
    assert!(r.engine.runtime.cooldowns.active(a, reset).is_none());
}
