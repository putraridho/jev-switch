//! The application-facing privacy audit log.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::turn::application::ports::PrivacyAudit;
use crate::turn::domain::privacy::audit::AuditRecord;

#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
    store: Arc<dyn PrivacyAudit>,
}

impl AuditLog {
    pub fn new(path: PathBuf, store: Arc<dyn PrivacyAudit>) -> Self {
        Self { path, store }
    }

    pub fn in_state_dir(dir: &Path, store: Arc<dyn PrivacyAudit>) -> Self {
        Self::new(dir.join("privacy-audit.jsonl"), store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(&self, record: &AuditRecord) {
        self.store.record(&self.path, record);
    }

    pub fn tail(&self, count: usize) -> Vec<AuditRecord> {
        self.store.tail(&self.path, count)
    }
}
