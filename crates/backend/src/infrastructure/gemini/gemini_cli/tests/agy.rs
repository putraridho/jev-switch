//! Antigravity's legacy headless stream-json shape, kept as regression evidence.

use super::*;
use super::super::stream_events::{tool_failure, uint};
use crate::infrastructure::claude::claude_cli::tool_summary::tool_summary;
use crate::infrastructure::gemini::gemini_cli::stream_events::finish_or_empty;

/// Assumes Antigravity counts like Gemini (input includes cache reads); the `max`
/// keeps the cached share from ever exceeding the total if it does not.
fn agy_usage(usage: &Value) -> Usage {
    let cached = uint(usage, "cache_read_tokens");
    Usage {
        input_tokens: uint(usage, "input_tokens").max(cached),
        cached_input_tokens: cached,
        output_tokens: uint(usage, "output_tokens"),
        context_tokens: None,
    }
}

/// Antigravity `--output-format stream-json`: `init`, `step_update` and a final
/// `result`. A logged-out run is reported to print the bare JSON envelope (no
/// `event` key) instead, so that shape is accepted too.
pub(super) fn agy_events(ev: &Value) -> Vec<Event> {
    match ev["event"].as_str() {
        Some("step_update") => agy_step(&ev["step_update"]),
        Some("result") => vec![agy_result(&ev["result"])],
        None if ev.get("status").is_some() => vec![agy_result(ev)],
        _ => vec![],
    }
}

/// Tool steps go `ACTIVE` then `DONE`; announce on `ACTIVE` so each call shows once.
/// Text deltas are skipped: the `result` carries the whole answer.
fn agy_step(step: &Value) -> Vec<Event> {
    let info = &step["tool_info"];
    match (step["step_type"].as_str(), step["state"].as_str()) {
        (Some("tool"), Some("ACTIVE")) => {
            let name = info["name"].as_str().or(step["tool_name"].as_str()).unwrap_or("tool");
            vec![Event::Tool(tool_summary(name, &info["parameters"]))]
        }
        (Some("tool"), Some("DONE")) => tool_failure(info["error"]["message"].as_str()).into_iter().collect(),
        _ => vec![],
    }
}

fn agy_result(result: &Value) -> Event {
    let status = result["status"].as_str().unwrap_or("");
    if status != "SUCCESS" {
        let message = result["error"].as_str().map(String::from).unwrap_or_else(|| {
            format!("antigravity ended with status {}", if status.is_empty() { "(none)" } else { status })
        });
        return Event::Failed { message };
    }
    finish_or_empty(
        result["response"].as_str().unwrap_or(""),
        result["conversation_id"].as_str().map(String::from),
        agy_usage(&result["usage"]),
        &denied_note(result),
    )
}

/// An empty answer after denied tool calls is the headless permission trap: the
/// CLI cannot ask, so it denies and still exits 0. Say so instead of just "empty".
fn denied_note(result: &Value) -> String {
    match result["denied_actions"].as_array().map(Vec::len) {
        Some(n) if n > 0 => format!(
            " ({n} action(s) were denied because headless mode cannot ask; allow them under permissions.allow in the Antigravity settings.json)"
        ),
        _ => String::new(),
    }
}

    #[test]
    fn agy_turn_parses_to_tool_and_finish() {
        let events = parse_all(&agy(), AGY_TURN);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0], Event::Tool("read_file /work/a.txt".into()));
        let Event::Finished(f) = &events[1] else { panic!("{:?}", events[1]) };
        assert_eq!(f.text, "It says hello.");
        assert_eq!(f.session_id.as_deref(), Some("c0ffee00-1111-2222-3333-444455556666"));
        assert_eq!(f.usage, Usage { input_tokens: 1200, cached_input_tokens: 800, output_tokens: 300, context_tokens: None });
    }

    #[test]
    fn agy_logged_out_envelope_is_auth() {
        let events = parse_all(&agy(), AGY_AUTH);
        let message = failed_message(&events);
        assert_eq!(message, "authentication failed or timed out");
        assert_eq!(agy().classify_error(&message), ErrorKind::Auth);
    }

    #[test]
    fn agy_quota_error_is_quota_with_the_clis_wording() {
        let message = failed_message(&parse_all(&agy(), AGY_QUOTA));
        assert_eq!(agy().classify_error(&message), ErrorKind::Quota { resets_at: Some("in 3 hours".into()) });
    }

    #[test]
    fn agy_success_with_empty_response_is_failed_and_empty() {
        let events = parse_all(&agy(), AGY_EMPTY);
        assert!(events.iter().all(|e| !matches!(e, Event::Finished(_))), "{events:?}");
        let message = failed_message(&events);
        assert_eq!(agy().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn agy_empty_after_denied_actions_says_why() {
        let message = failed_message(&parse_all(&agy(), AGY_EMPTY_DENIED));
        assert!(message.contains("1 action(s) were denied") && message.contains("permissions.allow"), "{message}");
        assert_eq!(agy().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn agy_non_success_statuses_fail() {
        let canceled = r#"{"event":"result","result":{"status":"CANCELED","response":""}}"#;
        assert_eq!(agy_events(&serde_json::from_str::<Value>(canceled).unwrap()), vec![Event::Failed { message: "antigravity ended with status CANCELED".into() }]);
        let waiting = r#"{"event":"result","result":{"response":"partial"}}"#;
        assert!(matches!(&agy_events(&serde_json::from_str::<Value>(waiting).unwrap())[..], [Event::Failed { message }] if message.contains("(none)")));
    }

    #[test]
    fn agy_failed_tool_step_gives_a_hint() {
        let line = r#"{"event":"step_update","step_update":{"step_type":"tool","state":"DONE","tool_info":{"name":"run_command","error":{"type":"DENIED","message":"needs permission"}}}}"#;
        assert_eq!(agy_events(&serde_json::from_str::<Value>(line).unwrap()), vec![Event::Hint("\u{2717} tool failed: needs permission".into())]);
    }
