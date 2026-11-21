//! Codex event parsing, fed by recorded fixtures.

use super::*;
use super::super::stream_events::{shell_script, started_events};

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
            Usage { input_tokens: 44970, cached_input_tokens: 36864, output_tokens: 159, context_tokens: Some(44970) }
        );
    }

    #[test]
    fn context_size_is_the_last_request_from_the_rollout_not_the_turn_total() {
        let root = crate::testkit::scratch_dir::unique_scratch_dir("codex-cli-context", "t");
        let day = root.join("2026/11/21");
        std::fs::create_dir_all(&day).unwrap();
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":44970},"last_token_usage":{"input_tokens":23500}}}}"#;
        std::fs::write(day.join("rollout-2026-11-21T00-00-00-01a1e5a6-9f98-7c21-99f5-6468805d7543.jsonl"), line).unwrap();
        let b = backend().with_sessions_dir(Some(root.clone()));
        let Some(Event::Finished(f)) = parse_all(&b, TURN1).pop() else { panic!() };
        assert_eq!((f.usage.input_tokens, f.usage.context_tokens), (44970, Some(23500)));
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
