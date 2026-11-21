//! A planner with the delegate tool for a multi-part top route.

use super::*;

// ------------------------------------------------------------ orchestration

/// A multi-part request: Jev's `multi_part` is above the cutoff.
fn multi(task: TaskType) -> ProviderWant {
    ProviderWant { orchestration: jev_judgment::domain::orchestration::Orchestration::new(0.9, 0.1), continues_p: 0.1, ..want(task) }
}

fn orchestrating(mut r: Rig) -> Rig {
    let setup = Setup { bin: Some(PathBuf::from("/bin/delegate_mcp")), config_dir: std::env::temp_dir() };
    r.engine = r.engine.with_orchestrate(true).with_orch_setup(Some(setup));
    r
}

#[tokio::test]
async fn orchestration_is_off_by_default_even_for_a_multi_part_top_route() {
    let mut r = rig(&[Claude, Codex]);
    assert!(!r.engine.orchestrate());
    let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert!(r.attached.lock().unwrap().is_empty());
    assert!(r.ui.routes.iter().all(|l| !l.contains("orchestrat")), "{:?}", r.ui.routes);
    assert!(!r.started()[0].prompt.contains("orchestration"));
}

#[tokio::test]
async fn a_multi_part_top_route_runs_on_a_planner_with_the_delegate_tool() {
    let mut r = orchestrating(rig(&[Claude, Codex]));
    let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    let started = r.started();
    let top = started[0].provider;
    assert!(matches!(top, Claude | Codex));
    assert!(find(top, &started[0].model).is_some_and(|m| m.tier == catalog::Tier::Top), "{}", started[0].model);
    let attached = r.attached.lock().unwrap().clone();
    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].0, top);
    assert_eq!(attached[0].1.name, "jev");
    assert!(started[0].prompt.contains("`delegate` tool") && started[0].prompt.contains("rebuild the whole importer"));
    assert!(r.ui.routes[0].contains("orchestrating") && r.ui.routes[0].contains("Jev picks the workers"), "{:?}", r.ui.routes);
}

#[tokio::test]
async fn orchestration_needs_both_multi_part_and_a_top_route() {
    let mut r = orchestrating(rig(&[Claude, Codex]));
    // Multi-part, but a Feature turn normally goes to an Everyday model.
    r.turn("add the endpoint, the page and the docs", multi(TaskType::Feature)).await;
    // A Top route, but not multi-part.
    r.turn("why does this deadlock", ProviderWant { continues_p: 0.1, ..want(TaskType::Review) }).await;
    assert!(r.attached.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_planner_is_never_a_provider_without_mcp_support() {
    // Review's first choice is Codex, then Gemini; with Codex missing the normal route is Gemini Argon, which cannot take the tool.
    let mut r = orchestrating(rig(&[Gemini, Claude]));
    let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(r.started()[0].provider, Claude, "the planner is the best Top model that can take the tool");
    assert_eq!(r.attached.lock().unwrap()[0].0, Claude);
    assert!(r.ui.routes[0].contains("gemini cannot take the delegate tool"), "{:?}", r.ui.routes);
}

#[tokio::test]
async fn with_no_planner_available_the_turn_runs_normally_and_says_why() {
    let mut r = orchestrating(rig(&[Gemini]));
    let report = r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert_eq!(r.started()[0].provider, Gemini);
    assert!(r.attached.lock().unwrap().is_empty());
    assert!(r.ui.routes[0].contains("orchestration skipped") && r.ui.routes[0].contains("cannot take the delegate tool"), "{:?}", r.ui.routes);
}

#[tokio::test]
async fn a_missing_delegate_binary_skips_orchestration_with_a_reason() {
    let mut r = rig(&[Claude, Codex]);
    r.engine = r.engine.with_orchestrate(true).with_orch_setup(None);
    r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert!(r.attached.lock().unwrap().is_empty());
    assert!(r.ui.routes[0].contains("orchestration skipped") && r.ui.routes[0].contains("delegate_mcp"), "{:?}", r.ui.routes);
}

#[tokio::test]
async fn a_pinned_provider_without_mcp_support_is_not_overridden_by_orchestration() {
    let mut r = orchestrating(rig(&[Gemini, Claude]));
    r.engine.set_pin(Pin::Provider(Gemini));
    r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    assert_eq!(r.started()[0].provider, Gemini, "a pin is never swapped");
    assert!(r.attached.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_mcp_config_file_for_claude_code_is_written_before_the_planner_starts() {
    let dir = std::env::temp_dir().join(format!("jev-turn-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = rig(&[Claude]);
    let setup = Setup { bin: Some(PathBuf::from("/bin/delegate_mcp")), config_dir: dir.clone() };
    r.engine = r.engine.with_orchestrate(true).with_orch_setup(Some(setup));
    r.turn("rebuild the whole importer", multi(TaskType::Review)).await;
    let path = r.attached.lock().unwrap()[0].1.config_path.clone();
    assert!(path.starts_with(&dir));
    let json: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["mcpServers"]["jev"]["command"], "/bin/delegate_mcp");
    std::fs::remove_dir_all(&dir).ok();
}
