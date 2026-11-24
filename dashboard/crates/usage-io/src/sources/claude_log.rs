//! The gateway's usage log as a source (DESIGN 6.1). The types and the descriptor are
//! fixed here; T03 writes the bodies.

use crate::probe::FactsCell;
use jev_usage_core::port::{
    Cost, FetchFuture, HistoryStore, SourceDescriptor, SourceError, SourceId, StoreError, Trigger,
    UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::Unavailable;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Reads Claude's windows from `usage.jsonl` (and `usage.jsonl.1`), read-only.
pub struct ClaudeGatewayLogSource {
    _path: Option<PathBuf>,
    _clock: Arc<dyn Clock>,
    _facts: Arc<FactsCell>,
}

impl ClaudeGatewayLogSource {
    /// `path` None (no home, no state directory) answers `NotConfigured(UseGateway)`. The
    /// clock decides when a carried window has expired (DESIGN 6.1).
    pub fn new(path: Option<PathBuf>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            _path: path,
            _clock: clock,
            _facts: facts,
        }
    }
}

impl UsageSource for ClaudeGatewayLogSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId::ClaudeGatewayLog,
            provider: ProviderKey::Claude,
            trigger: Trigger::OnSignal,
            cost: Cost::Free,
            timeout: Duration::from_secs(5),
            fresh_for: Duration::from_mins(10),
            needs_login: false,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(std::future::ready(Err(SourceError::Unavailable(
            Unavailable::Internal,
        ))))
    }
}

/// First-run history backfill: `<path>.1` then `<path>`, in batches of 512 readings through
/// `HistoryStore::record_all` (which applies the write rule). Returns samples stored.
pub fn backfill(path: &Path, store: &dyn HistoryStore) -> Result<u64, StoreError> {
    let _ = (path, store);
    Err(StoreError::Unavailable)
}
