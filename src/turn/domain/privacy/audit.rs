//! The privacy audit log: what happened to each prompt (sent, or kept back), never the prompt.
//! Audit records never include the prompt or secrets.

use std::fmt;

use crate::shared::ProviderId;
use super::outgoing::Outgoing;

/// What the audit log records about one thing that happened to a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditAction {
    /// The prompt went to the provider (with secrets blanked, when `redacted` is not empty).
    Sent,
    /// The policy kept the prompt from the provider.
    Blocked,
}

/// One line of the audit log. It never holds the prompt or a secret: only sizes, secret
/// kinds and the policy's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    use std::path::PathBuf;
    use std::sync::Arc;
    use crate::turn::application::privacy_audit::AuditLog;
    use crate::turn::infrastructure::privacy_audit::FilePrivacyAudit;
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
        let log = AuditLog::in_state_dir(&dir, Arc::new(FilePrivacyAudit));
        let out = prepare(&Policy::default(), "x DB_PASSWORD=hunter2hunter2");
        log.record(&AuditRecord::sent(1, ProviderId::Claude, "opus", &out));
        log.record(&AuditRecord::blocked(2, ProviderId::Grok, "local-only mode is on".into()));
        log.record(&AuditRecord::sent(3, ProviderId::Codex, "gpt", &prepare(&Policy::default(), "plain")));
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(raw.lines().next().unwrap(), r#"{"ts_ms":1,"action":"sent","provider":"claude","model":"opus","prompt_bytes":37,"redacted":["named-secret"],"reason":null}"#);
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
        assert!(AuditLog::new(dir.join("none.jsonl"), Arc::new(FilePrivacyAudit)).tail(5).is_empty());
    }
}
