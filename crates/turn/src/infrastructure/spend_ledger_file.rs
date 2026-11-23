//! The shared spend ledger as a JSON-lines file in the state folder: every jev-code and
//! delegate_mcp process appends one line per finished attempt, and reads the others' lines
//! back. The file rotates like the other logs, so old sessions fall away.

use std::path::Path;

use serde::{Deserialize, Serialize};

use jev_backend::{ProviderId, Usage};
use jev_shared::infrastructure::jsonl_log::append_line;
use jev_shared::Measurement;
use crate::application::ports::SpendLedger;
use crate::domain::spend_ledger::LedgerLine;

pub use crate::application::ports::SPEND_LEDGER_FILE as FILE;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Wire {
    Session { ts_ms: u64, project: String },
    Spend {
        ts_ms: u64,
        project: String,
        writer: String,
        provider: String,
        model: String,
        input_tokens: u64,
        cached_input_tokens: u64,
        output_tokens: u64,
        #[serde(default)]
        unreported: bool,
    },
}

impl From<&LedgerLine> for Wire {
    fn from(line: &LedgerLine) -> Self {
        match line {
            LedgerLine::Session { ts_ms, project } => Self::Session { ts_ms: *ts_ms, project: project.clone() },
            LedgerLine::Spend { ts_ms, project, writer, provider, model, usage } => Self::Spend {
                ts_ms: *ts_ms,
                project: project.clone(),
                writer: writer.clone(),
                provider: provider.as_str().to_string(),
                model: model.clone(),
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                output_tokens: usage.output_tokens,
                unreported: usage.measurement == Measurement::Unavailable,
            },
        }
    }
}

impl Wire {
    /// `None` for a line of a provider this version does not know.
    fn into_line(self) -> Option<LedgerLine> {
        Some(match self {
            Self::Session { ts_ms, project } => LedgerLine::Session { ts_ms, project },
            Self::Spend { ts_ms, project, writer, provider, model, input_tokens, cached_input_tokens, output_tokens, unreported } => LedgerLine::Spend {
                ts_ms,
                project,
                writer,
                provider: provider.parse::<ProviderId>().ok()?,
                model,
                usage: Usage {
                    measurement: if unreported { Measurement::Unavailable } else { Measurement::Reported },
                    input_tokens,
                    cached_input_tokens,
                    output_tokens,
                    context_tokens: None,
                },
            },
        })
    }
}

/// The ledger kept in a file.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileSpendLedger;

impl SpendLedger for FileSpendLedger {
    fn append(&self, path: &Path, line: &LedgerLine) {
        if let Ok(text) = serde_json::to_string(&Wire::from(line)) {
            append_line(&path.to_string_lossy(), &text);
        }
    }

    fn read(&self, path: &Path) -> Vec<LedgerLine> {
        let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
        text.lines().filter_map(|l| serde_json::from_str::<Wire>(l).ok()?.into_line()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip_and_unknown_or_broken_ones_are_skipped() {
        let path = jev_testkit::scratch_dir::scratch_dir("spend-ledger", "round").join(FILE);
        let ledger = FileSpendLedger;
        assert!(ledger.read(&path).is_empty(), "no file, no lines");
        let session = LedgerLine::Session { ts_ms: 5, project: "/p".into() };
        let spend = LedgerLine::Spend {
            ts_ms: 6,
            project: "/p".into(),
            writer: "w".into(),
            provider: ProviderId::DeepSeek,
            model: "deepseek-flash".into(),
            usage: Usage { input_tokens: 10, cached_input_tokens: 4, output_tokens: 2, ..Usage::default() },
        };
        let unreported = LedgerLine::Spend {
            ts_ms: 7,
            project: "/p".into(),
            writer: "w".into(),
            provider: ProviderId::Claude,
            model: "m".into(),
            usage: Usage { measurement: Measurement::Unavailable, ..Usage::default() },
        };
        for l in [&session, &spend, &unreported] {
            ledger.append(&path, l);
        }
        append_line(&path.to_string_lossy(), "not json");
        append_line(&path.to_string_lossy(), r#"{"kind":"spend","ts_ms":1,"project":"/p","writer":"w","provider":"nobody","model":"m","input_tokens":1,"cached_input_tokens":0,"output_tokens":0}"#);
        assert_eq!(ledger.read(&path), vec![session, spend, unreported]);
    }
}
