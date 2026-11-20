//! Catching a provider up on what the others did, and per-provider sessions.

use super::*;

#[tokio::test]
async fn catch_up_is_prepended_on_a_switch_but_not_when_staying_and_is_not_recorded() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    let b = if a == Claude { Codex } else { Claude };
    r.turn("first request", want(TaskType::Feature)).await;
    assert_eq!(r.started().last().unwrap().prompt, "first request", "staying on a fresh provider: no catch-up");

    r.script(a, QUOTA);
    r.turn("second request", want(TaskType::Feature)).await;
    let on_b = r.started().last().unwrap().clone();
    assert_eq!(on_b.provider, b);
    assert!(on_b.prompt.contains("first request") && on_b.prompt.contains("M src/a.rs"), "{}", on_b.prompt);
    assert!(asked(&on_b.prompt).ends_with("Now the user says:\nsecond request"), "{}", on_b.prompt);
    let recent = r.engine.sessions.recent_context(1, 200);
    assert!(recent.starts_with("user: second request"), "{recent}");

    r.turn("third request", want(TaskType::Feature)).await;
    assert_eq!(asked(&r.started().last().unwrap().prompt), "third request", "B is up to date: no catch-up");
}

#[tokio::test]
async fn a_failed_attempt_leaves_the_catch_up_owed_and_a_success_settles_it() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    let b = if a == Claude { Codex } else { Claude };
    r.turn("first request", new_task(TaskType::Feature)).await;
    assert_eq!(r.started().last().unwrap().provider, a);

    // A stalls on the second turn, so B answers it: A has now missed that turn.
    r.script(a, STALL);
    r.turn("second request", new_task(TaskType::Feature)).await;
    assert_eq!(r.started().last().unwrap().provider, b);

    // A is tried again and stalls again. It was handed the catch-up, but never
    // answered, so it is still owed it.
    r.turn("third request", new_task(TaskType::Feature)).await;
    let failed_try = r.started().into_iter().rfind(|s| s.provider == a).unwrap();
    assert!(failed_try.prompt.contains("second request"), "{}", failed_try.prompt);
    assert_eq!(r.engine.sessions.missed_turns(a), 2, "the stalled attempt did not count as delivery");

    // Routing stays with B for a while after a switch, so take B out of the turn.
    r.script(a, OK);
    r.script(b, STALL);
    r.turn("fourth request", new_task(TaskType::Feature)).await;
    let on_a = r.started().last().unwrap().clone();
    assert_eq!(on_a.provider, a);
    assert!(on_a.prompt.contains("second request") && on_a.prompt.contains("third request"), "{}", on_a.prompt);
    assert!(on_a.prompt.ends_with("Now the user says:\nfourth request"), "{}", on_a.prompt);

    r.turn("fifth request", new_task(TaskType::Feature)).await;
    let again = r.started().last().unwrap().clone();
    assert_eq!((again.provider, again.prompt.as_str()), (a, "fifth request"), "delivered once, not repeated");
}

#[tokio::test]
async fn session_ids_are_resumed_per_provider() {
    let mut r = rig(&[Claude]);
    r.turn("one", want(TaskType::Feature)).await;
    r.turn("two", want(TaskType::Feature)).await;
    let s = r.started();
    assert_eq!(s[0].session, None);
    assert_eq!(s[1].session.as_deref(), Some("sess-claude"));
}

#[test]
fn a_turn_keeps_what_was_added_while_it_ran() {
    assert_eq!(with_steered("fix the bug", &[]), "fix the bug");
    assert_eq!(
        with_steered("fix the bug", &["and run clippy".to_string(), "then commit".to_string()]),
        "fix the bug\n\n(added while it ran) and run clippy\n\n(added while it ran) then commit"
    );
}
