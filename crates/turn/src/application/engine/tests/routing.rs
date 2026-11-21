//! Which model a turn goes to: usable sets, pins, task rules, routing off.

use super::*;

#[tokio::test]
async fn a_provider_that_is_not_usable_is_never_started() {
    let mut r = rig(&[Claude]);
    for task in TaskType::ALL {
        r.turn(LONG, new_task(task)).await;
    }
    let calls = r.factory_calls.lock().unwrap().clone();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|p| *p == Claude), "{calls:?}");

    // A cooling provider is as unusable as one that is not installed.
    let mut r = rig(&[Claude, Codex]);
    r.engine.runtime.cool_down(Claude, None, NOW);
    for task in TaskType::ALL {
        r.turn(LONG, new_task(task)).await;
    }
    let calls = r.factory_calls.lock().unwrap().clone();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|p| *p == Codex), "{calls:?}");
}

#[tokio::test]
async fn nothing_usable_says_so_and_starts_nothing() {
    let mut r = rig(&[]);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute);
    assert!(r.factory_calls.lock().unwrap().is_empty());
    assert!(report.message.contains("/providers") && report.message.contains("feature"), "{}", report.message);
    assert_eq!(r.ui.notes.len(), 1);
    assert!(r.ui.routes.is_empty());
}

fn avoid_deepseek_when(task: &str) -> jev_routing::domain::user_rules::Rules {
    jev_routing::domain::user_rules::Rules::parse(&format!("avoid deepseek when task is {task}")).0
}

#[tokio::test]
async fn a_team_pin_still_answers_to_the_persons_task_rules() {
    // The repo pins deepseek; the person's rule forbids it for this turn's task.
    for task in [TaskType::Security, TaskType::Feature] {
        let mut r = rig(&[Claude, DeepSeek]);
        r.engine.guards.rules = avoid_deepseek_when(&task.to_string());
        r.engine.set_team_pin(Pin::Provider(DeepSeek));
        let report = r.turn("harden the auth flow", want(task)).await;
        assert_eq!(report.status, TurnStatus::Answered, "{task}: {}", report.message);
        assert_eq!(report.provider, Some(Claude), "{task}");
        assert!(r.started().iter().all(|s| s.provider != DeepSeek), "{task}");
        let route = r.ui.routes.last().cloned().unwrap_or_default();
        assert!(route.contains(".jev/team.json pin to deepseek skipped: your routing rules exclude deepseek"), "{task}: {route}");
    }
}

#[tokio::test]
async fn a_team_pin_that_cannot_serve_the_task_routes_automatically_and_says_so() {
    // No rules at all: deepseek is barred for security work by the catalog.
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.set_team_pin(Pin::Provider(DeepSeek));
    let report = r.turn("harden the auth flow", want(TaskType::Security)).await;
    assert_eq!(report.status, TurnStatus::Answered, "{}", report.message);
    assert_eq!(report.provider, Some(Claude));
    assert!(r.ui.routes[0].contains(".jev/team.json pin skipped: pinned provider deepseek"), "{:?}", r.ui.routes);
}

#[tokio::test]
async fn a_team_pin_to_a_cooling_provider_is_reported_not_swapped() {
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.runtime.cool_down(Claude, None, NOW);
    r.engine.set_team_pin(Pin::Provider(Claude));
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute, "{}", report.message);
    assert!(r.started().is_empty());
}

#[tokio::test]
async fn a_team_pin_to_an_unknown_model_is_reported_not_swapped() {
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.set_team_pin(Pin::Model(Claude, "nope".into()));
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute, "{}", report.message);
    assert!(r.started().is_empty());
}

#[tokio::test]
async fn a_team_pin_to_an_unusable_model_is_reported_not_swapped() {
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.runtime.unusable_models.insert((Claude, "claude-opus-5-5".to_string()));
    r.engine.set_team_pin(Pin::Model(Claude, "claude-opus-5-5".into()));
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute, "{}", report.message);
    assert!(r.started().is_empty());
}

#[tokio::test]
async fn a_team_pin_that_fails_during_the_turn_fails_over_to_automatic_routing() {
    // A pinned model that gives an empty answer is skipped for the rest of the turn.
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.set_team_pin(Pin::Model(Claude, "claude-opus-5-5".into()));
    r.script(Claude, EMPTY);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered, "{}", report.message);
    assert_eq!(report.provider, Some(DeepSeek));
    assert!(r.ui.routes.iter().any(|s| s.contains("pin") && s.contains("skipped")), "{:?}", r.ui.routes);

    // A pinned provider that hits its limit mid-turn is cooled down.
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.set_team_pin(Pin::Provider(Claude));
    r.script(Claude, QUOTA);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered, "{}", report.message);
    assert_eq!(report.provider, Some(DeepSeek));
}

#[tokio::test]
async fn a_person_pin_ignores_task_rules_but_a_team_pin_without_conflict_applies() {
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.guards.rules = avoid_deepseek_when("security");
    r.engine.set_team_pin(Pin::Provider(DeepSeek));
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.provider, Some(DeepSeek));
    assert!(r.ui.routes[0].contains("pinned by .jev/team.json"), "{:?}", r.ui.routes);

    // The person's own pin keeps its behaviour: rules step aside.
    let mut r = rig(&[Claude, DeepSeek]);
    r.engine.guards.rules = avoid_deepseek_when("feature");
    r.engine.set_pin(Pin::Provider(DeepSeek));
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.provider, Some(DeepSeek));
}

#[tokio::test]
async fn security_never_falls_back_to_deepseek() {
    let mut r = rig(&[DeepSeek]);
    let report = r.turn("audit the login flow", want(TaskType::Security)).await;
    assert_eq!(report.status, TurnStatus::NoRoute);
    assert!(report.message.contains("Security work never goes to deepseek"), "{}", report.message);
    assert!(r.factory_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_pin_that_becomes_unusable_is_reported_not_swapped() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.engine.set_pin(Pin::Provider(a));
    r.script(a, QUOTA);
    let report = r.turn("add a retry to the client", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::NoRoute);
    assert_eq!(r.started().len(), 1);
    assert!(report.message.contains("pin"), "{}", report.message);
}

#[tokio::test]
async fn routing_off_is_claude_only_with_one_attempt() {
    let mut r = rig(&[Codex]);
    r.engine.set_enabled(false);
    let strong = CodeWant { tier: 2, confident: true, reason: "confidently reasoning".into(), signals: Value::Null.into() };
    let report = r.engine.run_judged("why is this slow", Judged::Legacy(strong), &mut r.ui).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!((report.provider, report.model.as_deref()), (Some(Claude), Some(MODEL_STRONG)));
    assert!(r.factory_calls.lock().unwrap().iter().all(|p| *p == Claude));
    assert!(r.ui.routes[0].contains("claude") && r.ui.routes[0].contains(MODEL_STRONG));

    // An easier turn holds Opus (warm cache) instead of dropping at once.
    let easy = CodeWant { tier: 1, confident: true, reason: "everyday".into(), signals: Value::Null.into() };
    let report = r.engine.run_judged("thanks", Judged::Legacy(easy), &mut r.ui).await;
    assert_eq!(report.model.as_deref(), Some(MODEL_STRONG));

    // A failure is reported; there is no failover in this mode.
    r.script(Claude, QUOTA);
    let again = CodeWant { tier: 1, confident: true, reason: "everyday".into(), signals: Value::Null.into() };
    let before = r.started().len();
    let report = r.engine.run_judged("and now", Judged::Legacy(again), &mut r.ui).await;
    assert_eq!(report.status, TurnStatus::Failed);
    assert_eq!(r.started().len(), before + 1);
}

#[tokio::test]
async fn routing_off_honours_a_claude_pin_and_ignores_another_providers() {
    let mut r = rig(&[Claude]);
    let w = || CodeWant { tier: 2, confident: true, reason: "r".into(), signals: Value::Null.into() };
    r.engine.set_pin(Pin::parse("sonnet").unwrap());
    let report = r.engine.run_judged("x", Judged::Legacy(w()), &mut r.ui).await;
    assert_eq!(report.model.as_deref(), Some(MODEL_MID));
    r.engine.set_pin(Pin::Provider(Codex));
    let report = r.engine.run_judged("y", Judged::Legacy(w()), &mut r.ui).await;
    assert_eq!(report.provider, Some(Claude));
    assert!(r.ui.routes.last().unwrap().contains("ignored"));
}

#[tokio::test]
async fn a_missing_jev_judgment_still_routes_to_a_usable_model() {
    let mut r = rig(&[Claude]);
    let report = r.engine.run_judged("hello", Judged::Providers(None), &mut r.ui).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(report.provider, Some(Claude));
}

#[test]
fn legacy_tiers_are_catalog_models() {
    for (tier, id) in LEGACY_TIERS.iter().enumerate() {
        assert_eq!(legacy_spec(tier).id, *id);
    }
    assert_eq!(legacy_spec(9).id, MODEL_STRONG);
}

#[test]
fn signals_copy_every_field_of_the_want() {
    let mut w = want(TaskType::Docs);
    w.correction_p = Some(0.4);
    w.security_keywords = Security::Weak;
    let s = Signals::from(&w);
    assert_eq!((s.task, s.correction_p, s.security_keywords), (TaskType::Docs, Some(0.4), Security::Weak));
    assert_eq!((s.task_confidence, s.continues_p, s.edit_p), (0.9, 0.9, 0.3));
}
