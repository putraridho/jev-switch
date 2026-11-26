//! The gateway's usage log as a source (DESIGN 6.1). The types and the descriptor are
//! fixed here; T03 writes the bodies.

use crate::probe::FactsCell;
use crate::tail::{for_each_line, TailReader};
use jev_usage_core::meter::{Meter, Meters, Reading};
use jev_usage_core::parse::claude_log;
use jev_usage_core::port::{
    Cost, FetchFuture, HistoryStore, SourceDescriptor, SourceError, SourceId, StoreError, Trigger,
    UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::time::UnixMillis;
use jev_usage_core::usage::{Setup, Unavailable};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// Reads Claude's windows from `usage.jsonl` (and `usage.jsonl.1`), read-only.
pub struct ClaudeGatewayLogSource {
    state: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
    facts: Arc<FactsCell>,
}

impl ClaudeGatewayLogSource {
    /// `path` None (no home, no state directory) answers `NotConfigured(UseGateway)`. The
    /// clock decides when a carried window has expired (DESIGN 6.1).
    pub fn new(path: Option<PathBuf>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                tail: path.map(TailReader::new),
                initialized: false,
                windows: Vec::new(),
            })),
            clock,
            facts,
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
        let state = Arc::clone(&self.state);
        let now = self.clock.now();
        let plan = self.facts.get().plan(ProviderKey::Claude);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut state = state.lock().map_err(|_| SourceError::Unavailable(Unavailable::Internal))?;
                let State { tail, initialized, windows } = &mut *state;
                let tail = tail.as_mut().ok_or(SourceError::NotConfigured(Setup::UseGateway))?;
                let mut accept = |line: &[u8]| {
                    if let Some(sample) = claude_log::parse_line(line) {
                        for meter in sample.windows.as_slice() {
                            if let Some(existing) = windows.iter_mut().find(|(old, _)| old.id() == meter.id()) {
                                if sample.at > existing.1 { *existing = (meter.clone(), sample.at); }
                            } else { windows.push((meter.clone(), sample.at)); }
                        }
                        true
                    } else { false }
                };
                if *initialized { tail.read_new(&mut |line| { accept(line); }).map_err(SourceError::Unavailable)?; }
                else { tail.seek_newest(16 * 1024 * 1024, &mut accept).map_err(SourceError::Unavailable)?; *initialized = true; }
                windows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id().key().cmp(&b.0.id().key())));
                windows.truncate(8);
                let newest = windows.first().map(|(_, at)| *at);
                windows.retain(|(meter, at)| {
                    if Some(*at) == newest { return true; }
                    let future = matches!(meter, Meter::Window(w) if w.resets_at.is_some_and(|reset| reset > now.seconds()));
                    future && now.saturating_since(*at) < Duration::from_hours(168)
                });
                let observed_at = windows.iter().map(|(_, at)| *at).max().ok_or(SourceError::NotConfigured(Setup::UseGateway))?;
                let meters = Meters::new(windows.iter().map(|(meter, _)| meter.clone()).collect()).ok_or(SourceError::Unavailable(Unavailable::Internal))?;
                Ok(Reading { meters, plan, observed_at })
            }).await.map_err(|_| SourceError::Unavailable(Unavailable::Internal))?
        })
    }
}

/// First-run history backfill: `<path>.1` then `<path>`, in batches of 512 readings through
/// `HistoryStore::record_all` (which applies the write rule). Returns samples stored.
pub fn backfill(path: &Path, store: &dyn HistoryStore) -> Result<u64, StoreError> {
    let mut previous = path.as_os_str().to_os_string();
    previous.push(".1");
    let mut batch = Vec::with_capacity(512);
    let mut stored = 0_u64;
    let mut failure = None;
    for file in [PathBuf::from(previous), path.to_path_buf()] {
        if !file.exists() {
            continue;
        }
        for_each_line(&file, &mut |line| {
            if failure.is_some() {
                return;
            }
            if let Some(sample) = claude_log::parse_line(line) {
                batch.push(Reading {
                    meters: sample.windows,
                    plan: None,
                    observed_at: sample.at,
                });
                if batch.len() == 512 {
                    match store.record_all(ProviderKey::Claude, &batch) {
                        Ok(count) => stored = stored.saturating_add(count),
                        Err(error) => failure = Some(error),
                    }
                    batch.clear();
                }
            }
        })
        .map_err(|_| StoreError::Unavailable)?;
        if let Some(error) = failure {
            return Err(error);
        }
    }
    if !batch.is_empty() {
        stored = stored.saturating_add(store.record_all(ProviderKey::Claude, &batch)?);
    }
    Ok(stored)
}

struct State {
    tail: Option<TailReader>,
    initialized: bool,
    windows: Vec<(Meter, UnixMillis)>,
}
