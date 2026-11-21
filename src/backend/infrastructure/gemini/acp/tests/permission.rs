use super::*;
use crate::backend::Decision;

#[test]
fn tool_only_start_asks_host_then_finishes_with_real_answer() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    let events = acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"c",
        "name":"run_command","kind":"execute","rawInput":{"command":"git status --short"}})));
    assert!(matches!(&events[..], [Event::Tool(t)] if t.contains("git status")));
    let events = acp.parse(&permission());
    let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
    assert_eq!(req.tool, "Bash");
    assert_eq!(req.input, json!({"command":"git status --short"}));
    let reply: Value = serde_json::from_str(&acp.answer(req, &Decision::AllowAlways).unwrap()).unwrap();
    assert_eq!(reply["id"], "permission-1");
    assert_eq!(reply["result"]["outcome"]["optionId"], "yes");
    acp.parse(&update(
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private reasoning"}}),
    ));
    for text in ["Working tree ", "is clean."] {
        acp.parse(&update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}})));
    }
    let events = acp.parse(&json!({"id":5,"result":{"stopReason":"end_turn"}}));
    assert!(
        matches!(&events[..], [Event::Finished(f)] if f.text == "Working tree is clean." && f.session_id.as_deref() == Some("s"))
    );
}

#[test]
fn denial_and_missing_once_option_never_grant_permission() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Edit, None);
    for missing in [false, true] {
        let mut ev = permission();
        if missing {
            ev["params"]["options"] = json!([{"optionId":"forever","kind":"allow_always"}]);
        }
        let events = acp.parse(&ev);
        let [Event::PermissionRequest(req)] = &events[..] else { panic!() };
        let reply: Value =
            serde_json::from_str(&acp.answer(req, &Decision::Deny { reason: "no".into() }).unwrap()).unwrap();
        if missing {
            assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
        } else {
            assert_eq!(reply["result"]["outcome"]["optionId"], "no");
        }
    }
}


#[test]
fn titles_cannot_turn_unknown_tools_into_auto_approved_commands() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"x","title":"git status","kind":"execute"});
    let events = acp.parse(&ev);
    let [Event::PermissionRequest(req)] = &events[..] else { panic!() };
    assert_ne!(req.tool, "Bash");
}

#[test]
fn antigravitys_command_line_is_a_plain_command_shown_without_the_protocols_bookkeeping() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"x","name":"run_command","kind":"execute","status":"pending","title":"ls -la",
        "rawInput":{"CommandLine":"git status && ls -la","WaitMsBeforeAsync":5000,"toolSummary":"Check"}});
    let events = acp.parse(&ev);
    let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
    assert_eq!(req.tool, "Bash");
    assert_eq!(req.input, json!({"command":"git status && ls -la"}));
    assert_eq!(req.summary, "git status && ls -la");
    // The same command run in another directory is not judged as one run in the project.
    ev["params"]["toolCall"]["rawInput"]["Cwd"] = json!("/etc");
    let events = acp.parse(&ev);
    let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
    assert_ne!(req.tool, "Bash");
}

#[test]
fn a_tool_named_like_a_claude_tool_is_not_judged_by_that_tools_rules() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"z","name":"WebFetch","kind":"fetch","rawInput":{"url":"https://docs.rs/x"}});
    let events = acp.parse(&ev);
    let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
    assert_eq!(req.tool, "acp_WebFetch");
    let places = crate::approval::domain::grantable::Places::from_env(std::path::Path::new("/tmp"));
    assert!(crate::approval::domain::always_plan::always_plan(&req.tool, &req.input, None, &places).is_err());
}

#[test]
fn another_tool_shows_what_the_model_passed_and_nothing_else() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Edit, None);
    let mut ev = permission();
    ev["params"]["toolCall"] = json!({"toolCallId":"y","name":"write_to_file","kind":"edit","status":"pending","title":"Write notes",
        "rawInput":{"TargetFile":"/p/notes.md","CodeContent":"hello"}});
    let events = acp.parse(&ev);
    let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
    assert_eq!(req.tool, "write_to_file");
    for noise in ["toolCallId", "status=", "kind=", "sessionUpdate", "rawInput"] {
        assert!(!req.summary.contains(noise), "{noise}: {}", req.summary);
    }
    assert!(req.summary.contains("/p/notes.md") && req.summary.contains("hello"), "{}", req.summary);
}
