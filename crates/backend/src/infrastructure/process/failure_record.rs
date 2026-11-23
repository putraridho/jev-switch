//! The raw line behind a failed CLI attempt, kept for diagnosis.
//!
//! The turn log keeps only the message a backend extracts from a failure line. When that
//! message is one bare word (Grok's `cancelled`, 2026-11-23) the cause is lost. This writes
//! the line itself, redacted and cut short, to `provider-failures.jsonl` in the state folder.

use serde_json::{json, Value};
use tokio::process::Command;

use crate::domain::event::Event;
use jev_shared::domain::redact::redact;
use jev_shared::infrastructure::jsonl_log::{append, now_ms, path_from_env};
use jev_shared::ProviderId;

/// Most characters of a raw line kept.
const MAX_CHARS: usize = 4000;
const LOG_ENV: &str = "JEV_FAILURE_LOG";

/// A command-local override isolates scripted CLIs without changing the host's
/// environment. An empty or removed override disables recording for that command.
pub(super) fn path_for_command(command: &Command) -> Option<String> {
    if let Some((_, value)) = command.as_std().get_envs().find(|(name, _)| *name == LOG_ENV) {
        return value.filter(|path| !path.is_empty()).map(|path| path.to_string_lossy().into_owned());
    }
    if cfg!(test) {
        return None;
    }
    path_from_env(LOG_ENV, "provider-failures.jsonl").filter(|path| !path.is_empty())
}

/// The record for `line`, if it parsed to a failure.
fn record(provider: ProviderId, line: &str, events: &[Event]) -> Option<Value> {
    let message = events.iter().find_map(|e| match e {
        Event::Failed { message } => Some(message.as_str()),
        _ => None,
    })?;
    let masked = redact(line.trim());
    let raw: String = masked.chars().take(MAX_CHARS).collect();
    Some(json!({
        "ts_ms": now_ms(),
        "provider": provider.as_str(),
        "message": redact(message),
        "truncated": masked.chars().count() > MAX_CHARS,
        "raw": raw,
    }))
}

/// Append a failure to the turn's configured log. Never fails the caller.
pub(super) fn note(path: Option<&str>, provider: ProviderId, line: &str, events: &[Event]) {
    let Some(path) = path else { return };
    let Some(row) = record(provider, line, events) else { return };
    append(path, &row);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_local_log_settings_do_not_use_the_real_state_folder() {
        let mut command = Command::new("sh");
        command.env(LOG_ENV, "/tmp/jev-failure-test.jsonl");
        assert_eq!(path_for_command(&command).as_deref(), Some("/tmp/jev-failure-test.jsonl"));
        command.env(LOG_ENV, "");
        assert_eq!(path_for_command(&command), None);
        command.env_remove(LOG_ENV);
        assert_eq!(path_for_command(&command), None);
    }

    #[test]
    fn scripted_clis_disable_failure_logging_even_when_compiled_upstream() {
        use crate::application::ports::Backend;
        let backend = crate::testing::ScriptedCli::new(ProviderId::Claude, "echo FAIL:boom");
        let command = backend.command(&crate::domain::turn_request::TurnRequest {
            dir: std::env::temp_dir(), model: "m".into(), prompt: "hello".into(), images: vec![],
            session_id: None, mode: Default::default(), trust_project_settings: false,
        });
        let setting = command.as_std().get_envs().find(|(name, _)| *name == LOG_ENV).unwrap();
        assert_eq!(setting.1, Some(std::ffi::OsStr::new("")));
        assert_eq!(path_for_command(&command), None);
    }

    #[test]
    fn a_failure_line_is_kept_whole_with_secrets_masked() {
        let line = r#"{"type":"result","is_error":true,"result":"cancelled","key":"sk-abcdefghijklmnopqrstuvwxyz0123456789"}"#;
        let events = [Event::Failed { message: "cancelled".into() }];
        let row = record(ProviderId::Grok, line, &events).expect("a failure is recorded");
        assert_eq!(row["provider"], "grok");
        assert_eq!(row["message"], "cancelled");
        assert_eq!(row["truncated"], false);
        let raw = row["raw"].as_str().unwrap();
        assert!(raw.contains(r#""result":"cancelled""#), "{raw}");
        assert!(!raw.contains("sk-abcdefghij"), "secret kept: {raw}");
    }

    #[test]
    fn a_line_that_did_not_fail_is_not_recorded() {
        let finished = [Event::Steered("hi".into())];
        assert!(record(ProviderId::Grok, "{}", &[]).is_none());
        assert!(record(ProviderId::Grok, "{}", &finished).is_none());
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let line = "é".repeat(MAX_CHARS + 10);
        let events = [Event::Failed { message: "m".into() }];
        let row = record(ProviderId::Grok, &line, &events).unwrap();
        assert_eq!(row["truncated"], true);
        assert_eq!(row["raw"].as_str().unwrap().chars().count(), MAX_CHARS);
    }
}
