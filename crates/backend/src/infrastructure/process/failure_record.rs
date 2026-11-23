//! The raw line behind a failed CLI attempt, kept for diagnosis.
//!
//! The turn log keeps only the message a backend extracts from a failure line. When that
//! message is one bare word (Grok's `cancelled`, 2026-11-23) the cause is lost. This writes
//! the line itself, redacted and cut short, to `provider-failures.jsonl` in the state folder.

use serde_json::{json, Value};

use crate::domain::event::Event;
use jev_shared::domain::redact::redact;
use jev_shared::infrastructure::jsonl_log::{append, now_ms, path_from_env};
use jev_shared::ProviderId;

/// Most characters of a raw line kept.
const MAX_CHARS: usize = 4000;

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

/// Append the record for `line` when it was a failure. Never fails the caller, and does
/// nothing under `cargo test` so tests leave the real state folder alone.
pub(super) fn note(provider: ProviderId, line: &str, events: &[Event]) {
    if cfg!(test) {
        return;
    }
    let Some(row) = record(provider, line, events) else { return };
    if let Some(path) = path_from_env("JEV_FAILURE_LOG", "provider-failures.jsonl") {
        append(&path, &row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
