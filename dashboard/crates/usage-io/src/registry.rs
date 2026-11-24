//! The one list of sources (DESIGN 4.6). The type is frozen; T08 writes the bodies.

use crate::http::HttpClient;
use crate::probe::FactsCell;
use crate::proc::ProcRunner;
use crate::secrets::KeySources;
use crate::service::Backfill;
use crate::sources::{
    claude_live::ClaudeLiveSource, claude_log::ClaudeGatewayLogSource, codex::CodexAppServerSource,
    deepseek::DeepSeekBalanceSource, placeholder::PlaceholderSource,
};
use crate::store::MetaKey;
use crate::store::SqliteStore;
use jev_usage_core::port::UsageSource;
use jev_usage_core::provider::ProviderKey;
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
    let mut sources: Vec<Arc<dyn UsageSource>> = vec![
        Arc::new(ClaudeGatewayLogSource::new(
            i.usage_log.clone(),
            Arc::clone(&i.clock),
            Arc::clone(&i.facts),
        )),
        Arc::new(ClaudeLiveSource::new(
            Arc::clone(&i.runner),
            Arc::clone(&i.clock),
            Arc::clone(&i.facts),
        )),
        Arc::new(CodexAppServerSource::new(
            Arc::clone(&i.runner),
            Arc::clone(&i.clock),
            Arc::clone(&i.facts),
        )),
        Arc::new(DeepSeekBalanceSource::new(
            Arc::clone(&i.http),
            Arc::clone(&i.clock),
            Arc::clone(&i.keys),
        )),
    ];
    for provider in ProviderKey::ALL {
        if let Some(source) = PlaceholderSource::new(provider) {
            sources.push(Arc::new(source));
        }
    }
    sources
}

/// The first-run backfill job, or `None` when `MetaKey::BackfillDone` is set or there is no
/// log path. The job runs `sources::claude_log::backfill` and then sets the meta key.
pub fn backfill_job(usage_log: Option<PathBuf>, store: Arc<SqliteStore>) -> Option<Backfill> {
    let path = usage_log?;
    if !matches!(store.meta_get(MetaKey::BackfillDone), Ok(None)) {
        return None;
    }
    Some(Backfill {
        provider: ProviderKey::Claude,
        run: Box::new(move || {
            let count = crate::sources::claude_log::backfill(&path, store.as_ref())?;
            store.meta_set(MetaKey::BackfillDone, 1)?;
            Ok(count)
        }),
    })
}
