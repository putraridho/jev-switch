use super::*;

#[test]
fn normal_turn_parses_to_tool_text_finish() {
    let events = parse_all(TURN1);
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0], Event::Tool("read_file /tmp/spike/a.txt".into()));
    assert_eq!(events[1], Event::Text("The file says hello.".into()));
    let Event::Finished(f) = &events[2] else { panic!("{:?}", events[2]) };
    assert_eq!(f.text, "The file says hello.");
    assert_eq!(f.session_id.as_deref(), Some("0199c0de-7a3e-7c21-9b8f-3d5e6f7a8b9c"));
    // 932 uncached + 1500 cache reads + 200 cache writes.
    assert_eq!(
        f.usage,
        Usage { input_tokens: 2632, cached_input_tokens: 1500, output_tokens: 54, context_tokens: None }
    );
}


#[test]
fn empty_answer_still_finishes_so_the_runner_can_call_it_empty() {
    let events = parse_all(EMPTY);
    let [Event::Finished(f)] = &events[..] else { panic!("{events:?}") };
    assert!(f.text.is_empty());
    assert_eq!(f.usage.output_tokens, 0);
}

#[test]
fn failure_message_falls_back_to_text_then_subtype() {
    let msg = |v: &str| match &backend().parse_line(v)[..] {
        [Event::Failed { message }] => message.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(msg(r#"{"type":"result","is_error":true,"result":"boom"}"#), "boom");
    assert_eq!(msg(r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#), "grok failed: error_max_turns");
    assert_eq!(msg(r#"{"type":"result","is_error":true}"#), "grok reported an error");
}

#[test]
fn junk_and_housekeeping_lines_are_ignored() {
    assert!(backend().parse_line("not json").is_empty());
    assert!(backend().parse_line("").is_empty());
    assert!(backend().parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
    assert!(backend().parse_line(r#"{"type":"user","message":{"content":[]}}"#).is_empty());
}
