//! Gemini CLI event parsing, fed by recorded fixtures.

use super::*;

#[test]
fn consumption_requires_numeric_input_and_output_but_zero_is_reported() {
    for (metadata, measurement) in crate::infrastructure::usage_measurement::test_cases() {
        let cli = gemini();
        cli.parse_line(r#"{"type":"message","role":"assistant","content":"done"}"#);
        let mut line = serde_json::json!({"type":"result","status":"success"});
        if let Some(metadata) = metadata {
            line["stats"] = metadata;
        }
        let events = cli.parse_line(&line.to_string());
        let Event::Finished(finish) = &events[0] else { panic!("{events:?}") };
        assert_eq!(finish.usage, Usage { measurement, ..Usage::default() }, "{line}");
    }
}

    #[test]
    fn gemini_turn_parses_to_narration_tool_and_rebuilt_answer() {
        let events = parse_all(&gemini(), GEMINI_TURN);
        // Each written piece comes first, for the live screen; the whole text still follows.
        assert_eq!(events.len(), 7, "{events:?}");
        assert_eq!(events[0], Event::TextDelta("I'll read ".into()));
        assert_eq!(events[1], Event::TextDelta("the file.".into()));
        assert_eq!(events[2], Event::Text("I'll read the file.".into()));
        assert_eq!(events[3], Event::Tool("read_file /work/a.txt".into()));
        assert_eq!(events[4], Event::TextDelta("It says ".into()));
        assert_eq!(events[5], Event::TextDelta("hello.".into()));
        let Event::Finished(f) = &events[6] else { panic!("{:?}", events[6]) };
        // Only the text after the last tool call is the answer.
        assert_eq!(f.text, "It says hello.");
        assert_eq!(f.session_id.as_deref(), Some("6f1c2d3e-4a5b-4c6d-8e7f-0123456789ab"));
        assert_eq!(f.usage, Usage { input_tokens: 1200, cached_input_tokens: 800, output_tokens: 300, context_tokens: None, ..Usage::default() });
    }

    #[test]
    fn a_gemini_message_without_delta_is_not_a_streamed_piece() {
        let events = gemini().parse_line(r#"{"type":"message","role":"assistant","content":"whole"}"#);
        assert_eq!(events, vec![]);
    }

    #[test]
    fn gemini_state_does_not_leak_into_the_next_turn() {
        let b = gemini();
        b.parse_line(r#"{"type":"message","role":"assistant","content":"left over"}"#);
        let events = parse_all(&b, GEMINI_EMPTY_OK);
        assert_eq!(events, vec![Event::Failed { message: EMPTY_ANSWER.into() }]);
    }

    #[test]
    fn gemini_quota_error_carries_the_reset_time() {
        let events = parse_all(&gemini(), GEMINI_QUOTA);
        assert!(matches!(&events[0], Event::Text(t) if t.starts_with("! Attempt 1 failed")));
        let message = failed_message(&events);
        assert_eq!(gemini().classify_error(&message), ErrorKind::Quota { resets_at: Some("3h20m5s".into()) });
    }

    #[test]
    fn gemini_empty_reply_uses_the_earlier_error_event_and_is_empty() {
        let events = parse_all(&gemini(), GEMINI_EMPTY_ERR);
        let message = failed_message(&events);
        assert!(message.contains("empty response"), "{message}");
        assert_eq!(gemini().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn gemini_success_with_no_text_is_failed_not_finished() {
        let events = parse_all(&gemini(), GEMINI_EMPTY_OK);
        assert_eq!(events, vec![Event::Failed { message: EMPTY_ANSWER.into() }]);
        assert_eq!(gemini().classify_error(EMPTY_ANSWER), ErrorKind::Empty);
    }

    #[test]
    fn gemini_failed_tool_result_gives_a_hint() {
        let line = r#"{"type":"tool_result","tool_id":"x","status":"error","error":{"type":"POLICY","message":"denied by policy"}}"#;
        assert_eq!(gemini().parse_line(line), vec![Event::Hint("\u{2717} tool failed: denied by policy".into())]);
        let ok = r#"{"type":"tool_result","tool_id":"x","status":"success","output":"fine"}"#;
        assert!(gemini().parse_line(ok).is_empty());
    }

    #[test]
    fn gemini_error_result_without_any_message_still_fails() {
        let line = r#"{"type":"result","status":"error","stats":{}}"#;
        assert_eq!(gemini().parse_line(line), vec![Event::Failed { message: "gemini reported an error".into() }]);
    }

    #[test]
    fn junk_and_unrelated_lines_are_ignored_by_both() {
        for b in [gemini(), agy()] {
            assert!(b.parse_line("Loaded cached credentials.").is_empty());
            assert!(b.parse_line("").is_empty());
            assert!(b.parse_line(r#"{"type":"unknown"}"#).is_empty());
            assert!(b.parse_line(r#"{"event":"unknown"}"#).is_empty());
        }
    }
