use super::*;

#[test]
fn a_command_still_running_when_the_turn_ends_is_reported() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"c","name":"run_command","kind":"execute","rawInput":{"CommandLine":"npm test"}})));
    acp.parse(&update(json!({"sessionUpdate":"tool_call_update","toolCallId":"c","status":"in_progress"})));
    acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"d","name":"run_command","kind":"execute","rawInput":{"CommandLine":"ls"}})));
    acp.parse(&update(json!({"sessionUpdate":"tool_call_update","toolCallId":"d","status":"completed"})));
    let end = acp.parse(&json!({"id":5,"result":{"stopReason":"end_turn"}}));
    assert!(matches!(&end[..], [Event::Hint(h), _] if h.contains("1 command was left running")), "{end:?}");
}

#[test]
fn a_call_announced_by_name_alone_gets_its_row_when_its_command_arrives() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Auto, None);
    let first = acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"c","name":"run_command","kind":"execute","status":"pending"})));
    assert!(first.is_empty(), "no row from the name alone: {first:?}");
    let later = acp.parse(&update(json!({"sessionUpdate":"tool_call_update","toolCallId":"c","rawInput":{"CommandLine":"cargo test -q"}})));
    assert_eq!(later, vec![Event::Tool("Bash cargo test -q".into())]);
    let again = acp.parse(&update(json!({"sessionUpdate":"tool_call_update","toolCallId":"c","status":"completed"})));
    assert!(again.is_empty(), "one row per call");
    // A call that ends without saying more still shows, by name.
    acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"d","name":"manage_task"})));
    let ended = acp.parse(&update(json!({"sessionUpdate":"tool_call_update","toolCallId":"d","status":"completed"})));
    assert_eq!(ended, vec![Event::Tool("manage_task".into())]);
    let file = acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"e","name":"view_file","rawInput":{"AbsolutePath":"/p/a.rs"}})));
    assert_eq!(file, vec![Event::Tool("view_file /p/a.rs".into())]);
}
