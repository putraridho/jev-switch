//! Grok Build over ACP: modes it takes, edits passed on with their path, errors with their detail.

use super::*;

/// A Grok session up to the prompt: no modes advertised, `default` or `plan` set anyway.
fn grok_setup(acp: &mut Acp, mode: Mode) -> Vec<Value> {
    acp.start(&request(mode, None));
    acp.parse(&json!({"id":1,"result":{"protocolVersion":1}}));
    acp.drain();
    assert!(acp.parse(&json!({"id":2,"result":{"sessionId":"s","models":{"currentModelId":"grok-4.7"}}})).is_empty());
    let mut sent: Vec<Value> = acp.drain().iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    acp.parse(&json!({"id":3,"result":{}}));
    sent.extend(acp.drain().iter().map(|l| serde_json::from_str::<Value>(l).unwrap()));
    acp.parse(&json!({"id":4,"result":{}}));
    sent.extend(acp.drain().iter().map(|l| serde_json::from_str::<Value>(l).unwrap()));
    sent
}

#[test]
fn grok_takes_default_or_plan_without_advertising_modes() {
    for (mode, want) in [(Mode::Plan, "plan"), (Mode::Edit, "default"), (Mode::Auto, "default"), (Mode::Ask, "default")] {
        let mut acp = Acp::for_agent(Agent::Grok);
        let sent = grok_setup(&mut acp, mode);
        assert_eq!(sent[0]["method"], "session/set_model");
        assert_eq!(sent[1]["method"], "session/set_mode");
        assert_eq!(sent[1]["params"]["modeId"], want, "{mode}");
        assert_eq!(sent[2]["method"], "session/prompt");
    }
}

#[test]
fn the_agent_survives_a_new_turn() {
    let mut acp = Acp::for_agent(Agent::Grok);
    grok_setup(&mut acp, Mode::Edit);
    let sent = grok_setup(&mut acp, Mode::Plan);
    assert_eq!(sent[1]["params"]["modeId"], "plan");
}

#[test]
fn a_grok_edit_is_asked_about_as_edit_with_its_one_path() {
    let mut acp = Acp::for_agent(Agent::Grok);
    grok_setup(&mut acp, Mode::Edit);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"e","kind":"edit","title":"Edit a.rs",
        "locations":[{"path":"/w/a.rs"}],"rawInput":{"file_path":"/w/a.rs","old_string":"x","new_string":"y"}});
    let [Event::PermissionRequest(req)] = &acp.parse(&ev)[..] else { panic!() };
    assert_eq!(req.tool, "Edit");
    assert_eq!(req.input.str_field("file_path"), Some("/w/a.rs"));
}

#[test]
fn an_edit_with_no_path_or_two_paths_is_not_an_edit() {
    let mut acp = Acp::for_agent(Agent::Grok);
    grok_setup(&mut acp, Mode::Edit);
    for call in [
        json!({"toolCallId":"e","kind":"edit","title":"Edit"}),
        json!({"toolCallId":"e","kind":"edit","locations":[{"path":"/w/a.rs"}],"rawInput":{"file_path":"/etc/passwd"}}),
    ] {
        let mut ev = permission();
        ev["params"]["toolCall"] = call;
        let [Event::PermissionRequest(req)] = &acp.parse(&ev)[..] else { panic!() };
        assert_ne!(req.tool, "Edit");
    }
}

#[test]
fn antigravity_edits_are_never_mapped_to_edit() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Ask, None);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"e","kind":"edit","locations":[{"path":"/w/a.rs"}]});
    let [Event::PermissionRequest(req)] = &acp.parse(&ev)[..] else { panic!() };
    assert_ne!(req.tool, "Edit");
}

#[test]
fn a_grok_error_keeps_its_detail_so_a_quota_is_recognised() {
    let mut acp = Acp::for_agent(Agent::Grok);
    grok_setup(&mut acp, Mode::Edit);
    // As grok 1.0.92 answered `session/prompt` with its free usage spent (2026-11-21).
    let ev = json!({"jsonrpc":"2.0","id":5,"error":{"code":-32003,"message":"Rate limited","data":"API error (status 429 Too Many Requests): subscription:free-usage-exhausted: You've used all the included free usage for model grok-4.7 for now. Usage resets over a rolling 24-hour window"}});
    let [Event::Failed { message }] = &acp.parse(&ev)[..] else { panic!() };
    assert!(message.starts_with("grok ACP: Rate limited: API error (status 429"), "{message}");
    assert!(matches!(crate::classify_common(message), Some(crate::ErrorKind::Quota { .. })), "{message}");
}
