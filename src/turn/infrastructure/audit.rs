//! The audit log file: where `audit.jsonl` lives, how an entry is appended and read back,
//! and the entry for a finished turn. The entry types and the text built from them are in
//! [`crate::shared::domain::audit`].

use std::path::{Path, PathBuf};

use crate::shared::domain::audit::{preview, Entry, Event, Outcome};
use crate::shared::infrastructure::audit_wire::EntryWire;
use crate::session::domain::session_log::TurnRecord;
use crate::session::domain::turn_status::TurnStatus;

/// The file inside the state folder.
pub const FILE: &str = "audit.jsonl";

/// How a turn that ended with `s` is recorded in the audit log.
fn outcome_of(s: TurnStatus) -> Outcome {
    match s {
        TurnStatus::Answered => Outcome::Answered,
        TurnStatus::NoRoute => Outcome::NoRoute,
        TurnStatus::Failed => Outcome::Failed,
        TurnStatus::Cancelled => Outcome::Cancelled,
    }
}

/// The log entry for a finished turn.
pub fn turn_entry(ts_ms: u64, project: &str, t: &TurnRecord) -> Entry {
    let usage = t.usage.unwrap_or_default();
    Entry {
        ts_ms,
        project: project.to_string(),
        event: Event::Turn {
            model: t.model.clone(),
            outcome: outcome_of(t.status),
            elapsed_ms: t.elapsed_ms,
            in_tokens: usage.input_tokens,
            out_tokens: usage.output_tokens,
            prompt: preview(&t.prompt),
            cached_tokens: usage.cached_input_tokens,
            context_tokens: usage.context_tokens.unwrap_or(0),
            tool_calls: t.tool_calls,
        },
    }
}

/// `audit.jsonl` in `state_dir`.
pub fn path_in(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE)
}

/// Append `entry`; never fails the caller (an audit line is not worth a lost turn).
pub fn append(path: &Path, entry: &Entry) {
    if let Ok(line) = serde_json::to_string(&EntryWire::from(entry)) {
        crate::shared::infrastructure::jsonl_log::append_line(&path.to_string_lossy(), &line);
    }
}

/// Every entry in the file, oldest first; lines that are not entries are skipped.
pub fn read(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str::<EntryWire>(l).ok().map(Into::into)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Usage;
    use crate::shared::domain::audit::{audit_text, history, history_text, PREVIEW_CHARS};

    fn record(prompt: &str, model: &str, status: TurnStatus) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            answer: "SECRET answer text".into(),
            status,
            model: model.into(),
            route: None,
            usage: Some(Usage { input_tokens: 10, cached_input_tokens: 0, output_tokens: 4, context_tokens: None }),
            elapsed_ms: 2500,
            ended_ms: 0,
            tool_calls: 0,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-audit-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE)
    }

    #[test]
    fn a_turn_entry_keeps_a_redacted_preview_and_never_the_answer() {
        let e = turn_entry(1_000, "/p", &record("use ANTHROPIC_API_KEY=hunter2hunter2\nsecond line", "claude · s", TurnStatus::Answered));
        let line = serde_json::to_string(&EntryWire::from(&e)).unwrap();
        assert!(line.contains(r#""event":"turn""#) && line.contains("[REDACTED]"), "{line}");
        assert!(!line.contains("hunter2") && !line.contains("SECRET answer") && !line.contains("second line"), "{line}");
    }

    #[test]
    fn a_turn_entry_records_cache_context_and_tool_calls_and_old_lines_still_read() {
        let mut r = record("x", "claude · s", TurnStatus::Answered);
        r.usage = Some(Usage { input_tokens: 900, cached_input_tokens: 800, output_tokens: 5, context_tokens: Some(70_000) });
        r.tool_calls = 12;
        let line = serde_json::to_string(&EntryWire::from(&turn_entry(1, "/p", &r))).unwrap();
        assert_eq!(line, r#"{"ts_ms":1,"project":"/p","event":"turn","model":"claude · s","outcome":"answered","elapsed_ms":2500,"in_tokens":900,"out_tokens":5,"prompt":"x","cached_tokens":800,"context_tokens":70000,"tool_calls":12}"#);
        let Event::Turn { cached_tokens, context_tokens, tool_calls, .. } = Entry::from(serde_json::from_str::<EntryWire>(&line).unwrap()).event else { panic!("{line}") };
        assert_eq!((cached_tokens, context_tokens, tool_calls), (800, 70_000, 12));

        let old = r#"{"ts_ms":1,"project":"/p","event":"turn","model":"m","outcome":"answered","elapsed_ms":5,"in_tokens":1,"out_tokens":2,"prompt":"p"}"#;
        let Event::Turn { cached_tokens, context_tokens, tool_calls, .. } = Entry::from(serde_json::from_str::<EntryWire>(old).unwrap()).event else { panic!() };
        assert_eq!((cached_tokens, context_tokens, tool_calls), (0, 0, 0));
    }

    #[test]
    fn a_long_prompt_is_cut_on_a_character_boundary() {
        let p = preview(&"é".repeat(300));
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn entries_round_trip_through_the_file_and_bad_lines_are_skipped() {
        let path = temp("roundtrip");
        let _ = std::fs::remove_file(&path);
        let a = turn_entry(1, "/p", &record("one", "m", TurnStatus::Answered));
        let b = Entry { ts_ms: 2, project: "/p".into(), event: Event::Export { path: "a.md".into() } };
        append(&path, &a);
        std::fs::write(&path, format!("{}not json\n{{\"event\":\"from_the_future\"}}\n", std::fs::read_to_string(&path).unwrap())).unwrap();
        append(&path, &b);
        assert_eq!(read(&path), vec![a, b]);
        assert!(read(&path.with_file_name("missing.jsonl")).is_empty());
    }

    #[test]
    fn history_groups_by_utc_day_newest_first() {
        let day = 86_400_000;
        let entries = vec![
            turn_entry(day, "/p", &record("a", "m1", TurnStatus::Answered)),
            turn_entry(day + 5, "/p", &record("b", "m2", TurnStatus::Failed)),
            turn_entry(2 * day, "/p", &record("c", "m1", TurnStatus::Answered)),
            Entry { ts_ms: 2 * day, project: "/p".into(), event: Event::Profile { name: "strict".into() } },
        ];
        let days = history(&entries);
        assert_eq!(days.iter().map(|d| d.day.as_str()).collect::<Vec<_>>(), ["1970-01-03", "1970-01-02"]);
        assert_eq!((days[1].turns, days[1].failed, days[1].in_tokens, days[1].out_tokens), (2, 1, 20, 8));
        assert_eq!(days[1].by_model.get("m1"), Some(&1));
        assert!(history_text(&days).contains("1970-01-02  2 turns (1 failed)"));
    }

    #[test]
    fn audit_text_shows_the_newest_and_says_when_empty() {
        assert!(audit_text(&[], 5).contains("empty"));
        let entries: Vec<Entry> = (1..=4).map(|i| turn_entry(i * 1000, "/p", &record(&format!("task {i}"), "m", TurnStatus::Answered))).collect();
        let text = audit_text(&entries, 2);
        assert!(text.contains("last 2 of 4") && text.contains("task 3") && text.contains("task 4") && !text.contains("task 2"), "{text}");
    }
}
