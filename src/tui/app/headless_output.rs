//! `jev-code -p --output-format json`: the result of one turn as a single JSON object on
//! stdout, for scripts, CI and other programs to read. Progress still goes to stderr.
//!
//! The object is a typed struct ([`JsonResult`]); nothing here builds untyped JSON.

use std::str::FromStr;

use serde::Serialize;

use crate::changes::infrastructure::git_working_tree::FileChange;
use crate::turn::application::engine::{TurnReport, TurnStatus};
use crate::tui::views::status_facts as views;

/// How `-p` prints its result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Only the answer (the default).
    #[default]
    Text,
    /// One JSON object: status, answer, model, usage, files.
    Json,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!("unknown output format `{other}` (text or json)")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Answered,
    NoRoute,
    Failed,
    Cancelled,
}

impl From<TurnStatus> for Status {
    fn from(s: TurnStatus) -> Self {
        match s {
            TurnStatus::Answered => Self::Answered,
            TurnStatus::NoRoute => Self::NoRoute,
            TurnStatus::Failed => Self::Failed,
            TurnStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct UsageOut {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FileOut {
    path: String,
    added: u32,
    removed: u32,
}

impl From<&FileChange> for FileOut {
    fn from(f: &FileChange) -> Self {
        Self { path: f.path.clone(), added: f.added, removed: f.removed }
    }
}

/// What `--output-format json` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonResult {
    status: Status,
    exit_code: i32,
    /// The answer; empty unless the turn answered.
    answer: String,
    /// Why the turn did not answer; empty when it did.
    message: String,
    provider: Option<String>,
    model: Option<String>,
    attempts: u32,
    elapsed_ms: u64,
    usage: Option<UsageOut>,
    files: Vec<FileOut>,
}

impl JsonResult {
    pub fn of(report: &TurnReport, elapsed_ms: u64) -> Self {
        Self {
            status: report.status.into(),
            exit_code: views::exit_code(report.status),
            answer: report.text.trim_end().to_string(),
            message: report.message.clone(),
            provider: report.provider.map(|p| p.as_str().to_string()),
            model: report.model.clone(),
            attempts: report.attempts,
            elapsed_ms,
            usage: report.usage.map(|u| UsageOut { input_tokens: u.input_tokens, cached_input_tokens: u.cached_input_tokens, output_tokens: u.output_tokens }),
            files: report.changes.as_ref().map(|c| c.files.iter().map(FileOut::from).collect()).unwrap_or_default(),
        }
    }

    /// One line of JSON.
    pub fn line(&self) -> String {
        // Serializing plain strings and numbers cannot fail; the fallback keeps the output valid anyway.
        serde_json::to_string(self).unwrap_or_else(|_| "{\"status\":\"failed\",\"exit_code\":1}".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ProviderId, Usage};

    fn report(status: TurnStatus, text: &str) -> TurnReport {
        TurnReport {
            status,
            text: text.to_string(),
            message: String::new(),
            provider: Some(ProviderId::Claude),
            model: Some("claude-sonnet-5-5".into()),
            usage: Some(Usage { input_tokens: 10, cached_input_tokens: 4, output_tokens: 2, context_tokens: None }),
            attempts: 1,
            tool_calls: 0,
            changes: None,
        }
    }

    #[test]
    fn formats_parse_or_say_what_is_accepted() {
        assert_eq!("json".parse(), Ok(OutputFormat::Json));
        assert_eq!("text".parse(), Ok(OutputFormat::Text));
        assert_eq!("xml".parse::<OutputFormat>(), Err("unknown output format `xml` (text or json)".to_string()));
    }

    #[test]
    fn an_answer_is_one_line_with_status_model_and_usage() {
        let line = JsonResult::of(&report(TurnStatus::Answered, "done\n"), 1500).line();
        assert_eq!(
            line,
            "{\"status\":\"answered\",\"exit_code\":0,\"answer\":\"done\",\"message\":\"\",\"provider\":\"claude\",\"model\":\"claude-sonnet-5-5\",\"attempts\":1,\"elapsed_ms\":1500,\"usage\":{\"input_tokens\":10,\"cached_input_tokens\":4,\"output_tokens\":2},\"files\":[]}"
        );
    }

    #[test]
    fn a_failure_carries_its_message_and_exit_code() {
        let mut r = report(TurnStatus::Failed, "");
        r.message = "usage limit \"hit\"".into();
        let line = JsonResult::of(&r, 5).line();
        assert!(line.contains("\"status\":\"failed\",\"exit_code\":1") && line.contains("usage limit \\\"hit\\\""), "{line}");
    }
}
