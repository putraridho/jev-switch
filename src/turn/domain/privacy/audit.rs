//! The privacy audit log: what happened to each prompt (sent, or kept back), never the prompt.
//! The one part of the policy that writes to disk, so it has its own file.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::shared::ProviderId;
use super::outgoing::Outgoing;

/// What the audit log records about one thing that happened to a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// The prompt went to the provider (with secrets blanked, when `redacted` is not empty).
    Sent,
    /// The policy kept the prompt from the provider.
    Blocked,
}

/// One line of the audit log. It never holds the prompt or a secret: only sizes, secret
/// kinds and the policy's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub ts_ms: u64,
    pub action: AuditAction,
    pub provider: String,
    pub model: String,
    pub prompt_bytes: usize,
    pub redacted: Vec<String>,
    pub reason: Option<String>,
}

impl AuditRecord {
    pub fn sent(ts_ms: u64, provider: ProviderId, model: &str, out: &Outgoing) -> Self {
        Self {
            ts_ms,
            action: AuditAction::Sent,
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_bytes: out.prompt.len(),
            redacted: out.redacted.iter().map(|k| k.to_string()).collect(),
            reason: None,
        }
    }

    pub fn blocked(ts_ms: u64, provider: ProviderId, reason: String) -> Self {
        Self {
            ts_ms,
            action: AuditAction::Blocked,
            provider: provider.to_string(),
            model: String::new(),
            prompt_bytes: 0,
            redacted: Vec::new(),
            reason: Some(reason),
        }
    }
}

/// Append-only, one JSON line per record, readable only by its owner.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `<state>/privacy-audit.jsonl`.
    pub fn in_state_dir(dir: &Path) -> Self {
        Self::new(dir.join("privacy-audit.jsonl"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Never fails the turn: an audit line that cannot be written is dropped.
    pub fn record(&self, rec: &AuditRecord) {
        let Ok(mut line) = serde_json::to_string(rec) else { return };
        line.push('\n');
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        if let Ok(mut f) = opts.open(&self.path) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// The last `n` records, oldest first; lines that do not parse are skipped.
    pub fn tail(&self, n: usize) -> Vec<AuditRecord> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let all: Vec<AuditRecord> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let skip = all.len().saturating_sub(n);
        all.into_iter().skip(skip).collect()
    }
}

impl fmt::Display for AuditRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.action {
            AuditAction::Sent if self.redacted.is_empty() => write!(f, "sent to {} {} ({} bytes)", self.provider, self.model, self.prompt_bytes),
            AuditAction::Sent => write!(f, "sent to {} {} with {} secret(s) blanked: {}", self.provider, self.model, self.redacted.len(), self.redacted.join(", ")),
            AuditAction::Blocked => write!(f, "kept from {}: {}", self.provider, self.reason.as_deref().unwrap_or("policy")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::domain::privacy::outgoing::prepare;
    use crate::turn::domain::privacy::Policy;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-privacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_audit_log_records_sizes_and_kinds_never_the_prompt() {
        let dir = temp("audit");
        let log = AuditLog::in_state_dir(&dir);
        let out = prepare(&Policy::default(), "x DB_PASSWORD=hunter2hunter2");
        log.record(&AuditRecord::sent(1, ProviderId::Claude, "opus", &out));
        log.record(&AuditRecord::blocked(2, ProviderId::Grok, "local-only mode is on".into()));
        log.record(&AuditRecord::sent(3, ProviderId::Codex, "gpt", &prepare(&Policy::default(), "plain")));
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert!(!raw.contains("hunter2") && !raw.contains("DB_PASSWORD"));
        let tail = log.tail(2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].action, AuditAction::Blocked);
        assert_eq!(tail[1].to_string(), "sent to codex gpt (5 bytes)");
        assert_eq!(log.tail(10)[0].to_string(), "sent to claude opus with 1 secret(s) blanked: named-secret");
        assert_eq!(tail[0].to_string(), "kept from grok: local-only mode is on");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(log.path()).unwrap().permissions()) & 0o777, 0o600);
        // A missing log is an empty tail.
        assert!(AuditLog::new(dir.join("none.jsonl")).tail(5).is_empty());
    }
}
