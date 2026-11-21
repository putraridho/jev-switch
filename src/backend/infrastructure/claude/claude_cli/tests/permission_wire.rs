//! Host approvals: control requests and their responses.

use super::*;

#[test]
fn golden_claude_control_response() {
    const MASKED: &str = "# masked: nothing";
    const GOLDEN: &str = golden!("backend/claude_control_response.txt");
    let request = |input: Value, blocked_path: Option<&str>| PermissionRequest {
        id: "req-1".into(),
        tool: "Write".into(),
        summary: "unused".into(),
        input: crate::approval::infrastructure::tool_input_wire::tool_input_from_json(&input),
        blocked_path: blocked_path.map(String::from),
    };
    let full = request(json!({ "path": "/tmp/p/x.txt", "content": "a \"b\"\n", "nested": { "z": 1, "a": [1e2, 2.0] } }), Some("/tmp/p/x.txt"));
    let bare = request(json!({}), None);
    let line = |r: &PermissionRequest, d: Decision| session().permission_response(r, &d).unwrap();
    let actual = format!(
        "=== allow\n{}\n=== allow_always\n{}\n=== deny\n{}\n=== allow_empty_input\n{}\n",
        line(&full, Decision::Allow),
        line(&full, Decision::AllowAlways),
        line(&full, Decision::Deny { reason: "not here".into() }),
        line(&bare, Decision::Allow),
    );
    let (head, body) = GOLDEN.split_once('\n').unwrap();
    assert_eq!(head, MASKED);
    assert_eq!(body, actual);
}

#[test]
fn the_recorded_control_request_becomes_a_permission_request() {
    let r = request_from(ALLOW_TURN);
    assert_eq!(r.id, "2f318992-1d40-4a9f-afab-303ef045d9d6");
    assert_eq!(r.tool, "Bash");
    assert_eq!(r.summary, "touch /tmp/spike/outside.txt");
    assert_eq!(r.blocked_path.as_deref(), Some("/tmp/spike/outside.txt"));
    assert_eq!(r.input.str_field("command"), Some("touch /tmp/spike/outside.txt"));
    // The whole recorded turn: the request, and the denial event that old headless runs got is absent.
    let events: Vec<Event> = ALLOW_TURN.lines().flat_map(|l| backend().parse_line(l)).collect();
    assert_eq!(events.iter().filter(|e| matches!(e, Event::PermissionRequest(_))).count(), 1);
    assert!(matches!(events.last(), Some(Event::Finished(_))), "{events:?}");
}

#[test]
fn the_request_summary_and_debug_leave_secrets_out_of_logs() {
    let line = json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"curl -H 'Authorization: Bearer SECRET'"}}}).to_string();
    let Event::PermissionRequest(r) = &backend().parse_line(&line)[0] else { panic!() };
    assert!(r.summary.contains("SECRET"), "the person sees what they are approving");
    let debug = format!("{r:?}");
    assert!(!debug.contains("SECRET") && debug.contains("Bash") && debug.contains("r1"), "{debug}");
}

#[test]
fn writes_are_summarised_by_path_and_size_not_contents() {
    let line = json!({"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/p/.claude/settings.local.json","content":"{\"permissions\": {}}"}}}).to_string();
    let Event::PermissionRequest(r) = &backend().parse_line(&line)[0] else { panic!() };
    assert_eq!(r.summary, "/p/.claude/settings.local.json (19 bytes)");
}

#[test]
fn other_control_requests_and_malformed_ones_are_not_questions() {
    assert!(backend().parse_line(r#"{"type":"control_request","request_id":"x","request":{"subtype":"hook_callback"}}"#).is_empty());
    assert!(backend().parse_line(r#"{"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#).is_empty());
    assert!(backend().parse_line(r#"{"type":"control_cancel_request"}"#).is_empty(), "no id, nothing to withdraw");
}

#[test]
fn a_cancel_request_withdraws_its_request_id() {
    // The shape the SDK control protocol documents; the CLI does not send it in the recorded turns.
    let line = json!({"type":"control_cancel_request","request_id":"2f318992-1d40-4a9f-afab-303ef045d9d6"}).to_string();
    assert_eq!(
        backend().parse_line(&line),
        vec![Event::PermissionWithdrawn { id: "2f318992-1d40-4a9f-afab-303ef045d9d6".into() }]
    );
}

#[test]
fn allow_answers_with_the_original_input_as_the_recorded_line_does() {
    let r = request_from(ALLOW_TURN);
    for decision in [Decision::Allow, Decision::AllowAlways] {
        let line = session().permission_response(&r, &decision).unwrap();
        assert!(!line.contains('\n'), "one line");
        assert_eq!(json_of(&line), json_of(ALLOW_LINE));
    }
}

#[test]
fn deny_answers_with_the_reason_as_the_recorded_line_does() {
    let r = request_from(DENY_TURN);
    let line = session().permission_response(&r, &Decision::Deny { reason: "User denied via host".into() }).unwrap();
    assert_eq!(json_of(&line), json_of(DENY_LINE));
}
