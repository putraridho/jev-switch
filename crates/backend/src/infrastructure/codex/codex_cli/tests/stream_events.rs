//! Codex event parsing, fed by recorded fixtures.

use super::*;
use super::super::stream_events::{shell_script, started_events};

#[test]
fn consumption_requires_numeric_input_and_output_but_zero_is_reported() {
    for (metadata, measurement) in crate::infrastructure::usage_measurement::test_cases() {
        let mut line = json!({"type":"turn.completed"});
        if let Some(metadata) = metadata {
            line["usage"] = metadata;
        }
        let events = backend().parse_line(&line.to_string());
        let Event::Finished(finish) = &events[0] else { panic!("{events:?}") };
        assert_eq!(finish.usage, Usage { measurement, ..Usage::default() }, "{line}");
        let usage = super::super::stream_events::usage_of(&line["usage"], Some(50_000), Some(475_000));
        assert_eq!(
            usage,
            Usage {
                measurement,
                context_tokens: Some(50_000),
                window_tokens: Some(475_000),
                ..Usage::default()
            }
        );
    }
}

    #[test]
    fn first_turn_parses_to_tools_text_and_finish() {
        let all = parse_all(&backend(), TURN1);
        assert!(all.contains(&Event::ToolResult { ok: true, text: "hello\n".into() }), "a command's output is reported: {all:?}");
        let events: Vec<Event> = all.into_iter().filter(|e| !matches!(e, Event::ToolResult { .. })).collect();
        assert_eq!(events.len(), 5, "{events:?}");
        assert_eq!(events[0], Event::Text("I\u{2019}ll read `a.txt`, then append `world` on a new line.".into()));
        assert_eq!(events[1], Event::Tool("shell cat a.txt".into()));
        assert!(matches!(&events[2], Event::Tool(s) if s.starts_with("shell printf ") && s.ends_with(">> a.txt")), "{:?}", events[2]);
        assert_eq!(events[3], Event::Text("hello".into()));
        let Event::Finished(f) = &events[4] else { panic!("{:?}", events[4]) };
        // The LAST agent_message is the answer; the first one was only narration.
        assert_eq!(f.text, "hello");
        assert_eq!(f.session_id.as_deref(), Some("01a1e5a6-9f98-7c21-99f5-6468805d7543"));
        assert_eq!(
            f.usage,
            Usage { input_tokens: 44970, cached_input_tokens: 36864, output_tokens: 159, context_tokens: None, ..Usage::default() }
        );
    }

    #[test]
    fn missing_rollout_leaves_context_unknown_without_changing_consumption() {
        let root = jev_testkit::scratch_dir::unique_scratch_dir("codex-cli-missing-context", "t");
        for sessions_dir in [None, Some(root.clone())] {
            let codex = backend().with_sessions_dir(sessions_dir);
            codex.parse_line(r#"{"type":"thread.started","thread_id":"missing"}"#);
            let line = r#"{"type":"turn.completed","usage":{"input_tokens":2400000,"cached_input_tokens":2000000,"output_tokens":159}}"#;
            let events = codex.parse_line(line);
            let Event::Finished(finish) = &events[0] else { panic!("{events:?}") };
            assert_eq!(finish.usage, Usage { input_tokens: 2_400_000, cached_input_tokens: 2_000_000, output_tokens: 159, context_tokens: None, ..Usage::default() });
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn context_size_is_the_last_request_from_the_rollout_not_the_turn_total() {
        let root = jev_testkit::scratch_dir::unique_scratch_dir("codex-cli-context", "t");
        let day = root.join("2026/11/21");
        std::fs::create_dir_all(&day).unwrap();
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":44970},"last_token_usage":{"input_tokens":23500}}}}"#;
        std::fs::write(day.join("rollout-2026-11-21T00-00-00-01a1e5a6-9f98-7c21-99f5-6468805d7543.jsonl"), line).unwrap();
        let b = backend().with_sessions_dir(Some(root.clone()));
        let Some(Event::Finished(f)) = parse_all(&b, TURN1).pop() else { panic!() };
        assert_eq!(f.usage, Usage { input_tokens: 44970, cached_input_tokens: 36864, output_tokens: 159, context_tokens: Some(23500), ..Usage::default() });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_commands_are_not_announced_twice() {
        let tools = parse_all(&backend(), TURN1).into_iter().filter(|e| matches!(e, Event::Tool(_))).count();
        assert_eq!(tools, 2);
    }

    #[test]
    fn resumed_turn_keeps_the_thread_id_and_answers() {
        let events = parse_all(&backend(), TURN2);
        let Some(Event::Finished(f)) = events.last() else { panic!("{events:?}") };
        assert_eq!(f.text, "world");
        assert_eq!(f.session_id.as_deref(), Some("01a1e5a6-9f98-7c21-99f5-6468805d7543"));
        assert_eq!(f.usage.input_tokens, 60251);
        assert_eq!(f.usage.cached_input_tokens, 47616);
        assert_eq!(f.usage.output_tokens, 164);
    }

    #[test]
    fn a_new_thread_does_not_inherit_the_previous_answer() {
        let b = backend();
        parse_all(&b, TURN1);
        let events = b.parse_line(r#"{"type":"thread.started","thread_id":"t2"}"#);
        assert!(events.is_empty());
        let Event::Finished(f) = &b.parse_line(r#"{"type":"turn.completed","usage":{}}"#)[0] else { panic!() };
        assert_eq!((f.text.as_str(), f.session_id.as_deref()), ("", Some("t2")));
    }

    #[test]
    fn model_not_on_plan_fails_only_on_turn_failed() {
        let b = backend();
        let events = parse_all(&b, ERR_PLAN);
        // The warning item and the top-level `error` line produce nothing.
        let message = "The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account.";
        assert_eq!(events, vec![Event::Failed { message: message.into() }]);
        assert_eq!(b.classify_error(message), ErrorKind::NotEntitled);
    }

    #[test]
    fn reconnecting_lines_are_not_failures() {
        let line = r#"{"type":"error","message":"Reconnecting... 2/5 (stream disconnected)"}"#;
        assert!(backend().parse_line(line).is_empty());
    }

    #[test]
    fn turn_failed_without_a_message_still_fails() {
        let evs = backend().parse_line(r#"{"type":"turn.failed"}"#);
        assert_eq!(evs, vec![Event::Failed { message: "codex reported an error".into() }]);
    }

    #[test]
    fn file_changes_are_shown_as_edits_unless_they_failed() {
        let ok = r#"{"type":"item.completed","item":{"type":"file_change","status":"completed","changes":[{"path":"/p/a.rs","kind":"update"},{"path":"/p/b.rs","kind":"add"}]}}"#;
        assert_eq!(backend().parse_line(ok), vec![Event::Tool("edit /p/a.rs".into()), Event::Tool("edit /p/b.rs".into())]);
        let failed = ok.replace("completed\"", "failed\"");
        assert!(backend().parse_line(&failed).is_empty());
    }

    #[test]
    fn junk_and_unknown_lines_are_ignored() {
        let b = backend();
        assert!(b.parse_line("not json").is_empty());
        assert!(b.parse_line("").is_empty());
        assert!(b.parse_line(r#"{"type":"turn.started"}"#).is_empty());
        assert_eq!(b.parse_line(r#"{"type":"item.completed","item":{"type":"reasoning","text":"x"}}"#), vec![Event::Thinking("x".into())], "reasoning is shown apart, never as the answer");
    }

    #[test]
    fn shell_wrapper_is_shortened_and_capped() {
        assert_eq!(shell_script("/usr/bin/bash -lc 'cat a.txt'"), "cat a.txt");
        assert_eq!(shell_script("/usr/bin/bash -lc \"echo 'x'\""), "echo 'x'");
        assert_eq!(shell_script("ls -la"), "ls -la");
        let long = format!("/usr/bin/bash -lc '{}'", "x".repeat(300));
        let Event::Tool(s) = &started_events(&json!({"type":"command_execution","command":long}))[0] else { panic!() };
        assert_eq!(s.chars().count(), "shell ".len() + 100);
    }

    #[test]
    fn tool_calls_web_searches_and_subagents_are_announced_and_completed() {
        let b = backend();
        let mcp_start = r#"{"type":"item.started","item":{"type":"mcp_tool_call","tool":"delegate","arguments":{"brief":"write parser"}}}"#;
        assert_eq!(b.parse_line(mcp_start), vec![Event::Tool("delegate write parser".into())]);
        let mcp_done = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","tool":"delegate","status":"completed","result":"worker w1: done"}}"#;
        assert_eq!(b.parse_line(mcp_done), vec![Event::ToolResult { ok: true, text: "worker w1: done".into() }]);

        let search_start = r#"{"type":"item.started","item":{"type":"web_search","query":"rust 1.83 release"}}"#;
        assert_eq!(b.parse_line(search_start), vec![Event::Tool("web_search rust 1.83 release".into())]);
        let search_done = r#"{"type":"item.completed","item":{"type":"web_search","status":"completed","result":"found"}}"#;
        assert_eq!(b.parse_line(search_done), vec![Event::ToolResult { ok: true, text: "found".into() }]);

        let agent_start = r#"{"type":"item.started","item":{"type":"spawn_agent","description":"test worker"}}"#;
        assert_eq!(b.parse_line(agent_start), vec![Event::Tool("spawn_agent test worker".into())]);
        let agent_done = r#"{"type":"item.completed","item":{"type":"spawn_agent","status":"completed","result":"done"}}"#;
        assert_eq!(b.parse_line(agent_done), vec![Event::ToolResult { ok: true, text: "done".into() }]);
    }

    #[test]
    fn a_command_the_sandbox_blocked_gets_one_hint_and_only_while_the_network_is_closed() {
        let line = |out: &str| format!(r#"{{"type":"item.completed","item":{{"type":"command_execution","status":"failed","exit_code":1,"aggregated_output":{out:?}}}}}"#);
        let blocked = line("Error: listen EPERM: operation not permitted 127.0.0.1");
        let hints = |events: &[Event]| events.iter().filter(|e| matches!(e, Event::Hint(_))).count();

        let closed = backend();
        let first = closed.parse_line(&blocked);
        assert!(matches!(first[0], Event::ToolResult { ok: false, .. }), "{first:?}");
        assert_eq!(hints(&first), 1, "{first:?}");
        assert_eq!(hints(&closed.parse_line(&blocked)), 0, "said once per turn");
        assert_eq!(hints(&backend().parse_line(&line("chmod: x: Operation not permitted"))), 0, "no network word");
        assert_eq!(hints(&backend().parse_line(&line("assertion failed"))), 0);
        assert_eq!(hints(&backend().with_network(true).parse_line(&blocked)), 0, "network is open, so not the sandbox");
    }
