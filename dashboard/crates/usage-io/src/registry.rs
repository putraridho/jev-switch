//! The one list of sources (DESIGN 4.6). The type is frozen; T08 writes the bodies.

use crate::http::HttpClient;
use crate::probe::FactsCell;
use crate::proc::ProcRunner;
use crate::secrets::KeySources;
use crate::service::Backfill;
use crate::store::SqliteStore;
use jev_usage_core::port::UsageSource;
use jev_usage_core::time::Clock;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything the sources are built from.
pub struct RegistryInputs {
    /// The gateway's usage log, when a path could be worked out.
    pub usage_log: Option<PathBuf>,
    /// The shared login facts.
    pub facts: Arc<FactsCell>,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// The process spawner.
    pub runner: Arc<ProcRunner>,
    /// The HTTP client.
    pub http: Arc<HttpClient>,
    /// Where keys come from at fetch time; production passes `KeySources::system`.
    pub keys: Arc<dyn Fn() -> KeySources + Send + Sync>,
}

/// The one list of sources. Adding a provider is one line here. The live-check source is
/// always in the list; whether it may run is the setting, checked by the service at call time.
pub fn build(i: &RegistryInputs) -> Vec<Arc<dyn UsageSource>> {
    let _ = i;
    Vec::new()
}

/// The first-run backfill job, or `None` when `MetaKey::BackfillDone` is set or there is no
/// log path. The job runs `sources::claude_log::backfill` and then sets the meta key.
pub fn backfill_job(usage_log: Option<PathBuf>, store: Arc<SqliteStore>) -> Option<Backfill> {
    let _ = (usage_log, store);
    None
}
