//! The privacy audit's JSON-lines file adapter.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::turn::application::ports::PrivacyAudit;
use crate::turn::domain::privacy::audit::{AuditAction, AuditRecord};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActionWire {
    Sent,
    Blocked,
}

#[derive(Serialize, Deserialize)]
struct RecordWire {
    ts_ms: u64,
    action: ActionWire,
    provider: String,
    model: String,
    prompt_bytes: usize,
    redacted: Vec<String>,
    reason: Option<String>,
}

impl From<&AuditRecord> for RecordWire {
    fn from(record: &AuditRecord) -> Self {
        Self {
            ts_ms: record.ts_ms,
            action: match record.action { AuditAction::Sent => ActionWire::Sent, AuditAction::Blocked => ActionWire::Blocked },
            provider: record.provider.clone(),
            model: record.model.clone(),
            prompt_bytes: record.prompt_bytes,
            redacted: record.redacted.clone(),
            reason: record.reason.clone(),
        }
    }
}

impl From<RecordWire> for AuditRecord {
    fn from(record: RecordWire) -> Self {
        Self {
            ts_ms: record.ts_ms,
            action: match record.action { ActionWire::Sent => AuditAction::Sent, ActionWire::Blocked => AuditAction::Blocked },
            provider: record.provider,
            model: record.model,
            prompt_bytes: record.prompt_bytes,
            redacted: record.redacted,
            reason: record.reason,
        }
    }
}

#[derive(Debug)]
pub struct FilePrivacyAudit;

impl PrivacyAudit for FilePrivacyAudit {
    fn record(&self, path: &Path, record: &AuditRecord) {
        let Ok(mut line) = serde_json::to_string(&RecordWire::from(record)) else { return };
        line.push('\n');
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        if let Ok(mut file) = opts.open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn tail(&self, path: &Path, count: usize) -> Vec<AuditRecord> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let all: Vec<AuditRecord> = text.lines().filter_map(|line| serde_json::from_str::<RecordWire>(line).ok().map(Into::into)).collect();
        let skip = all.len().saturating_sub(count);
        all.into_iter().skip(skip).collect()
    }
}
