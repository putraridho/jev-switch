//! The audit log and the usage history built from it: one typed JSON line per thing
//! `jev-code` did that is worth answering for later (a turn, an export, a profile change),
//! appended to `audit.jsonl` in the state folder.
//!
//! The log keeps only a short, redacted preview of each prompt (see [`crate::redact`]), never
//! an answer, so it can be shared with a team lead or an auditor. Reading skips lines it does
//! not understand, so a newer version's entries never break an older one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::redact::redact;
use crate::sessionlog::{utc_stamp, TurnRecord};
use crate::turn::TurnStatus;

/// How much of a prompt the log keeps, in characters.
const PREVIEW_CHARS: usize = 120;

/// The file inside the state folder.
pub const FILE: &str = "audit.jsonl";

/// How a turn ended, as written in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Answered,
    NoRoute,
    Failed,
    Cancelled,
}

impl From<TurnStatus> for Outcome {
    fn from(s: TurnStatus) -> Self {
        match s {
            TurnStatus::Answered => Outcome::Answered,
            TurnStatus::NoRoute => Outcome::NoRoute,
            TurnStatus::Failed => Outcome::Failed,
            TurnStatus::Cancelled => Outcome::Cancelled,
        }
    }
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Turn { model: String, outcome: Outcome, elapsed_ms: u64, in_tokens: u64, out_tokens: u64, prompt: String },
    Export { path: String },
    Profile { name: String },
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub ts_ms: u64,
    pub project: String,
    #[serde(flatten)]
    pub event: Event,
}

/// The first line of a prompt, redacted and cut to [`PREVIEW_CHARS`].
pub fn preview(prompt: &str) -> String {
    let first = redact(prompt.lines().next().unwrap_or("").trim());
    if first.chars().count() <= PREVIEW_CHARS {
        return first;
    }
    let cut: String = first.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}…")
}

/// The log entry for a finished turn.
pub fn turn_entry(ts_ms: u64, project: &str, t: &TurnRecord) -> Entry {
    let usage = t.usage.unwrap_or_default();
    Entry {
        ts_ms,
        project: project.to_string(),
        event: Event::Turn {
            model: t.model.clone(),
            outcome: t.status.into(),
            elapsed_ms: t.elapsed_ms,
            in_tokens: usage.input_tokens,
            out_tokens: usage.output_tokens,
            prompt: preview(&t.prompt),
        },
    }
}

/// `audit.jsonl` in `state_dir`.
pub fn path_in(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE)
}

/// Append `entry`; never fails the caller (an audit line is not worth a lost turn).
pub fn append(path: &Path, entry: &Entry) {
    if let Ok(line) = serde_json::to_string(entry) {
        crate::log::append_line(&path.to_string_lossy(), &line);
    }
}

/// Every entry in the file, oldest first; lines that are not entries are skipped.
pub fn read(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Tokens and turns of one UTC day.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Day {
    pub day: String,
    pub turns: u64,
    pub failed: u64,
    pub in_tokens: u64,
    pub out_tokens: u64,
    pub by_model: BTreeMap<String, u64>,
}

/// The turns grouped by UTC day, newest day first.
pub fn history(entries: &[Entry]) -> Vec<Day> {
    let mut days: BTreeMap<String, Day> = BTreeMap::new();
    for e in entries {
        let Event::Turn { model, outcome, in_tokens, out_tokens, .. } = &e.event else { continue };
        let day = utc_stamp(e.ts_ms).0[..10].to_string();
        let d = days.entry(day.clone()).or_insert_with(|| Day { day, ..Day::default() });
        d.turns += 1;
        d.failed += u64::from(matches!(outcome, Outcome::Failed | Outcome::NoRoute));
        d.in_tokens += in_tokens;
        d.out_tokens += out_tokens;
        if !model.is_empty() {
            *d.by_model.entry(model.clone()).or_default() += 1;
        }
    }
    days.into_values().rev().collect()
}

fn describe(e: &Entry) -> String {
    let when = utc_stamp(e.ts_ms).0;
    match &e.event {
        Event::Turn { model, outcome, elapsed_ms, prompt, .. } => {
            let who = if model.is_empty() { "-" } else { model.as_str() };
            format!("{when}  turn    {outcome:?} · {who} · {:.1}s  {prompt}", *elapsed_ms as f64 / 1000.0)
        }
        Event::Export { path } => format!("{when}  export  {path}"),
        Event::Profile { name } => format!("{when}  profile {name}"),
    }
}

/// `/audit [n]`: the newest `n` entries, oldest of them first.
pub fn audit_text(entries: &[Entry], n: usize) -> String {
    if entries.is_empty() {
        return "  The audit log is empty: nothing has been recorded yet.".to_string();
    }
    let shown = &entries[entries.len().saturating_sub(n)..];
    let mut lines = vec![format!("  Audit log: the last {} of {} entries", shown.len(), entries.len())];
    lines.extend(shown.iter().map(|e| format!("  {}", describe(e))));
    lines.join("\n")
}

/// `/history`: turns and tokens per day.
pub fn history_text(days: &[Day]) -> String {
    if days.is_empty() {
        return "  No usage history yet: no turn has been recorded.".to_string();
    }
    let mut lines = vec!["  Usage history (UTC days, newest first)".to_string()];
    for d in days {
        let models: Vec<String> = d.by_model.iter().map(|(m, n)| format!("{m} x{n}")).collect();
        lines.push(format!("  {}  {} turns ({} failed)  {} in / {} out tokens  {}", d.day, d.turns, d.failed, d.in_tokens, d.out_tokens, models.join(", ")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Usage;

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
        let line = serde_json::to_string(&e).unwrap();
        assert!(line.contains(r#""event":"turn""#) && line.contains("[REDACTED]"), "{line}");
        assert!(!line.contains("hunter2") && !line.contains("SECRET answer") && !line.contains("second line"), "{line}");
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
