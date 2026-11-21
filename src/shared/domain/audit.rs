//! The audit log and the usage history built from it: one typed JSON line per thing
//! `jev-code` did that is worth answering for later (a turn, an export, a profile change),
//! appended to `audit.jsonl` in the state folder (the file side is `turn::infrastructure::audit`).
//!
//! The log keeps only a short, redacted preview of each prompt (see [`crate::shared::domain::redact`]), never
//! an answer, so it can be shared with a team lead or an auditor. Reading skips lines it does
//! not understand, so a newer version's entries never break an older one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::shared::domain::redact::redact;
use crate::shared::domain::utc::utc_stamp;

/// How much of a prompt the log keeps, in characters.
pub const PREVIEW_CHARS: usize = 120;

/// How a turn ended, as written in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Answered,
    NoRoute,
    Failed,
    Cancelled,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Turn {
        model: String,
        outcome: Outcome,
        elapsed_ms: u64,
        in_tokens: u64,
        out_tokens: u64,
        prompt: String,
        /// The cached share of `in_tokens`. Older lines and providers that do not say: 0.
        #[serde(default)]
        cached_tokens: u64,
        /// The size of the turn's last request, when the CLI reports it: 0 when it does not.
        #[serde(default)]
        context_tokens: u64,
        /// Tool calls of the attempt that ended the turn. Older lines: 0.
        #[serde(default)]
        tool_calls: u32,
    },
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
