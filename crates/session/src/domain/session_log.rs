//! What this session's turns were, kept in memory by the REPL: the source of `/cost`
//! (tokens and time by model), `/export` (the conversation as Markdown) and `/copy` (the
//! last answer). Pure data and formatting; the REPL fills it after every turn.

use jev_backend::Usage;
use crate::domain::turn_status::TurnStatus;

/// One turn as the REPL saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRecord {
    pub prompt: String,
    /// The answer; empty unless the turn answered.
    pub answer: String,
    pub status: TurnStatus,
    /// `provider · model` that answered or failed last; empty when no model was started.
    pub model: String,
    /// The same as ids, to look the model up in the catalog.
    pub route: Option<(jev_backend::ProviderId, String)>,
    pub usage: Option<Usage>,
    pub elapsed_ms: u64,
    /// When the turn ended, in ms since 1970 (the cache window counts from it).
    pub ended_ms: u64,
    /// Tool calls the model made in the attempt that ended the turn.
    pub tool_calls: u32,
}

/// Tokens and time of one model over the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelTotal {
    pub model: String,
    pub turns: usize,
    pub in_tokens: u64,
    pub cached_tokens: u64,
    pub out_tokens: u64,
    pub elapsed_ms: u64,
    pub unreported_turns: usize,
}

/// The turns of this session, oldest first.
#[derive(Debug, Clone, Default)]
pub struct SessionLog {
    turns: Vec<TurnRecord>,
}

impl SessionLog {
    pub fn push(&mut self, record: TurnRecord) {
        self.turns.push(record);
    }

    /// `/new`, `/clear`: forget the turns.
    pub fn clear(&mut self) {
        self.turns.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    pub fn turns(&self) -> &[TurnRecord] {
        &self.turns
    }

    /// The newest answer, for `/copy`.
    pub fn last_answer(&self) -> Option<&str> {
        self.turns.iter().rev().map(|t| t.answer.as_str()).find(|a| !a.trim().is_empty())
    }

    /// Tokens and time per model, in the order each was first used, then the sum of all.
    pub fn totals(&self) -> (Vec<ModelTotal>, ModelTotal) {
        let mut rows: Vec<ModelTotal> = Vec::new();
        let mut all = ModelTotal { model: "total".into(), ..ModelTotal::default() };
        for t in self.turns.iter().filter(|t| !t.model.is_empty()) {
            let at = match rows.iter().position(|r| r.model == t.model) {
                Some(i) => i,
                None => {
                    rows.push(ModelTotal { model: t.model.clone(), ..ModelTotal::default() });
                    rows.len() - 1
                }
            };
            for row in [&mut rows[at], &mut all] {
                row.turns += 1;
                row.elapsed_ms += t.elapsed_ms;
                if let Some(u) = &t.usage.filter(|usage| usage.measurement == jev_shared::Measurement::Reported) {
                    row.in_tokens += u.input_tokens;
                    row.cached_tokens += u.cached_input_tokens;
                    row.out_tokens += u.output_tokens;
                } else {
                    row.unreported_turns += 1;
                }
            }
        }
        (rows, all)
    }

    /// The conversation as Markdown: each prompt as a quote, each answer as written, with
    /// a dim line saying which model answered.
    pub fn to_markdown(&self, project: &str, started: &str) -> String {
        let mut out = format!("# jev-code session\n\n- Project: `{project}`\n- Exported: {started}\n- Turns: {}\n", self.turns.len());
        for (i, t) in self.turns.iter().enumerate() {
            out.push_str(&format!("\n## {}. You\n\n", i + 1));
            for line in t.prompt.lines() {
                out.push_str(&format!("> {line}\n"));
            }
            let who = if t.model.is_empty() { "jev-code".to_string() } else { t.model.clone() };
            out.push_str(&format!("\n### {who}\n\n"));
            let body = match t.status {
                TurnStatus::Answered => t.answer.trim_end().to_string(),
                TurnStatus::Cancelled => "_(stopped)_".to_string(),
                TurnStatus::Failed => "_(the turn failed)_".to_string(),
                TurnStatus::NoRoute => "_(no model could take the turn)_".to_string(),
            };
            out.push_str(&body);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(model: &str, answer: &str, tokens: (u64, u64, u64)) -> TurnRecord {
        TurnRecord {
            prompt: "fix it\nplease".into(),
            answer: answer.into(),
            status: if answer.is_empty() { TurnStatus::Cancelled } else { TurnStatus::Answered },
            model: model.into(),
            route: None,
            usage: Some(Usage { input_tokens: tokens.0, cached_input_tokens: tokens.1, output_tokens: tokens.2, context_tokens: None, ..Usage::default() }),
            elapsed_ms: 1000,
            ended_ms: 0,
            tool_calls: 0,
        }
    }

    #[test]
    fn totals_add_up_by_model_in_first_use_order() {
        let mut log = SessionLog::default();
        log.push(record("codex · g", "a", (10, 5, 1)));
        log.push(record("claude · s", "b", (100, 50, 10)));
        log.push(record("codex · g", "c", (20, 0, 2)));
        log.push(TurnRecord { model: String::new(), usage: None, ..record("", "", (0, 0, 0)) });
        let (rows, all) = log.totals();
        assert_eq!(rows.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(), ["codex · g", "claude · s"]);
        assert_eq!((rows[0].turns, rows[0].in_tokens, rows[0].out_tokens), (2, 30, 3));
        assert_eq!((all.turns, all.in_tokens, all.cached_tokens, all.out_tokens, all.elapsed_ms), (3, 130, 55, 13, 3000));
    }

    #[test]
    fn totals_distinguish_unreported_consumption_from_a_reported_zero() {
        let mut log = SessionLog::default();
        log.push(record("gemini", "known zero", (0, 0, 0)));
        log.push(TurnRecord { usage: Some(Usage { measurement: jev_shared::Measurement::Unavailable, ..Usage::default() }), ..record("gemini", "unknown", (0, 0, 0)) });
        log.push(TurnRecord { usage: None, ..record("gemini", "", (0, 0, 0)) });
        let (rows, total) = log.totals();
        assert_eq!(rows[0].turns, 3);
        assert_eq!(rows[0].unreported_turns, 2);
        assert_eq!(total.unreported_turns, 2);
    }

    #[test]
    fn the_last_answer_skips_turns_without_one() {
        let mut log = SessionLog::default();
        assert_eq!(log.last_answer(), None);
        log.push(record("m", "first", (0, 0, 0)));
        log.push(record("m", "", (0, 0, 0)));
        assert_eq!(log.last_answer(), Some("first"));
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn markdown_quotes_prompts_and_names_the_model() {
        let mut log = SessionLog::default();
        log.push(record("claude · s", "Done.\n", (0, 0, 0)));
        log.push(record("codex · g", "", (0, 0, 0)));
        let md = log.to_markdown("/p", "2026-11-17 10:00");
        assert!(md.starts_with("# jev-code session\n\n- Project: `/p`"), "{md}");
        assert!(md.contains("## 1. You\n\n> fix it\n> please\n\n### claude · s\n\nDone.\n"), "{md}");
        assert!(md.contains("### codex · g\n\n_(stopped)_"), "{md}");
    }
}
