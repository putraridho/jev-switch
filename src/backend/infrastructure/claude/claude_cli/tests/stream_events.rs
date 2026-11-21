//! Stream parsing: recorded turns, deltas, tool results, thinking, errors.

use super::*;
use crate::backend::infrastructure::claude::claude_cli::tool_summary::tool_summary;
use crate::backend::Usage;

#[test]
fn recorded_turn_parses_to_tool_plan_text_finish() {
    let all = parse_all(&backend());
    assert!(all.contains(&Event::ToolResult { ok: true, text: "1\thello\n2\t\n3\tworld\n4\t".into() }), "{all:?}");
    let events: Vec<Event> = all.into_iter().filter(|e| !matches!(e, Event::ToolResult { .. })).collect();
    assert_eq!(events.len(), 4, "{events:?}");
    assert!(matches!(&events[0], Event::Tool(s) if s.starts_with("Read ") && s.ends_with("/spike/a.txt")));
    assert!(matches!(&events[1], Event::Plan(p)
        if p.window == "seven_day" && (p.utilization - 0.29).abs() < 1e-9 && p.resets_at == Some(1795114800)));
    assert_eq!(events[2], Event::Text("hello".into()));
    let Event::Finished(f) = &events[3] else { panic!("{:?}", events[3]) };
    assert_eq!(f.text, "hello");
    assert_eq!(f.session_id.as_deref(), Some("03ede922-7bce-402e-b690-5271724d91f6"));
    // Turn total is 4 + 35536 + 14188; the context is the last iteration only: 2 + 24812 + 100.
    assert_eq!(
        f.usage,
        Usage { input_tokens: 49728, cached_input_tokens: 35536, output_tokens: 85, context_tokens: Some(24914) }
    );
}

#[test]
fn subagent_text_is_never_answer_text() {
    let line = json!({"type":"assistant","parent_tool_use_id":"agent-1","message":{"content":[
        {"type":"text","text":"subagent report"}, {"type":"tool_use","name":"Read","input":{"file_path":"a"}}
    ]}});
    assert_eq!(backend().parse_line(&line.to_string()), vec![Event::Tool("Read a".into())]);
    let result = json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","content":"subagent report"}]}});
    let events = backend().parse_line(&result.to_string());
    assert!(!events.iter().any(|e| matches!(e, Event::Text(_))), "a sub-agent's report is a tool result, not the answer: {events:?}");
}

#[test]
fn partial_text_is_the_live_preview_and_never_a_subagent_or_thinking() {
    let line = json!({"type":"stream_event","parent_tool_use_id":null,
        "event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}});
    assert_eq!(backend().parse_line(&line.to_string()), vec![Event::TextDelta("Hel".into())]);
    for event in [
        json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}),
        json!({"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{}"}}),
        json!({"type":"message_start"}),
    ] {
        let line = json!({"type":"stream_event","parent_tool_use_id":null,"event":event});
        assert_eq!(backend().parse_line(&line.to_string()), vec![], "not answer text: {line}");
    }
    let nested = json!({"type":"stream_event","parent_tool_use_id":"agent-1",
        "event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sub"}}});
    assert_eq!(backend().parse_line(&nested.to_string()), vec![], "a subagent's text never joins the answer");
}

#[test]
fn context_falls_back_to_usage_without_iterations() {
    let line = json!({"type":"result","result":"ok","usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}});
    let Event::Finished(f) = &backend().parse_line(&line.to_string())[0] else { panic!() };
    assert_eq!(f.usage.context_tokens, Some(6));
}

#[test]
fn error_result_is_failed() {
    let line = json!({"type":"result","is_error":true,"result":"boom"}).to_string();
    assert_eq!(backend().parse_line(&line), vec![Event::Failed { message: "boom".into() }]);
}

#[test]
fn permission_denied_gives_a_hint_that_is_not_answer_text() {
    let line = json!({"type":"system","subtype":"permission_denied","tool_name":"Bash"}).to_string();
    let evs = backend().parse_line(&line);
    assert!(matches!(&evs[..], [Event::Hint(t)] if t.contains("denied Bash") && t.contains("CODE_ALLOWED_TOOLS")));
}

#[test]
fn limit_notice_text_is_not_shown() {
    let line = json!({"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your session limit"}]}});
    assert!(backend().parse_line(&line.to_string()).is_empty());
}

#[test]
fn junk_and_init_lines_are_ignored() {
    assert!(backend().parse_line("not json").is_empty());
    assert!(backend().parse_line("").is_empty());
    assert!(backend().parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
    assert!(backend().parse_line(r#"{"type":"rate_limit_event","rate_limit_info":{}}"#).is_empty());
}

#[test]
fn tool_results_and_edits_of_the_main_conversation_are_reported() {
    let b = backend();
    let ok = json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"line one\nline two"}]}});
    assert_eq!(b.parse_line(&ok.to_string()), vec![Event::ToolResult { ok: true, text: "line one\nline two".into() }]);
    let failed = json!({"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"no such file"}]}]}});
    assert_eq!(b.parse_line(&failed.to_string()), vec![Event::ToolResult { ok: false, text: "no such file".into() }]);
    let wrapped = json!({"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"<tool_use_error>Error: No such tool available: Grep</tool_use_error>"}]}});
    assert_eq!(user_events(&wrapped), vec![Event::ToolResult { ok: false, text: "Error: No such tool available: Grep".into() }]);
    let nested = json!({"type":"user","parent_tool_use_id":"agent","message":{"content":[{"type":"tool_result","content":"x"}]}});
    assert!(b.parse_line(&nested.to_string()).is_empty(), "a sub-agent's results stay inside it");
    let edit = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"src/a.rs","old_string":"let a = 1;","new_string":"let a = 2;\nlet b = 3;"}}]}});
    assert_eq!(
        b.parse_line(&edit.to_string()),
        vec![Event::Tool("Edit src/a.rs".into()), Event::Edit { path: "src/a.rs".into(), removed: vec!["let a = 1;".into()], added: vec!["let a = 2;".into(), "let b = 3;".into()] }]
    );
    let write = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"n.txt","content":"hi\n"}}]}});
    assert!(matches!(&b.parse_line(&write.to_string())[1], Event::Edit { removed, added, .. } if removed.is_empty() && added == &vec!["hi".to_string()]));
}

#[test]
fn thinking_is_reported_apart_from_the_answer() {
    let line = json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"The empty case returns early."},{"type":"text","text":"Fixed."}]}});
    assert_eq!(backend().parse_line(&line.to_string()), vec![Event::Thinking("The empty case returns early.".into()), Event::Text("Fixed.".into())]);
}

#[test]
fn tool_summary_picks_the_telling_field() {
    assert_eq!(tool_summary("Bash", &json!({"command":"cargo test"})), "Bash cargo test");
    assert_eq!(tool_summary("Task", &json!({})), "Task");
    let long = "x".repeat(300);
    assert_eq!(tool_summary("Read", &json!({"file_path": long})).chars().count(), 105);
}

#[test]
fn classifies_common_errors() {
    assert!(matches!(backend().classify_error("You've hit your limit"), ErrorKind::Quota { .. }));
    assert_eq!(backend().classify_error("weird"), ErrorKind::Other);
}
