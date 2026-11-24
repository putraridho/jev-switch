//! The opt-in Claude live check (DESIGN 6.2): one tiny `claude -p` run, only ever on the
//! user's click. The type and the descriptor are fixed here; T04 writes the body.

use crate::probe::FactsCell;
use crate::proc::ProcRunner;
use jev_usage_core::port::{
    Cost, FetchFuture, SourceDescriptor, SourceError, SourceId, Trigger, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::Unavailable;
use std::sync::Arc;
use std::time::Duration;

/// Reads Claude's windows from the `rate_limit_event` lines of one `claude -p` run.
pub struct ClaudeLiveSource {
    _runner: Arc<ProcRunner>,
    _clock: Arc<dyn Clock>,
    _facts: Arc<FactsCell>,
}

impl ClaudeLiveSource {
    /// A source that starts `claude` through `runner` when the login facts say `Ready`.
    pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            _runner: runner,
            _clock: clock,
            _facts: facts,
        }
    }
}

impl UsageSource for ClaudeLiveSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId::ClaudeLive,
            provider: ProviderKey::Claude,
            trigger: Trigger::Manual,
            cost: Cost::SpendsQuota,
            timeout: Duration::from_mins(1),
            fresh_for: Duration::from_mins(10),
            needs_login: true,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(std::future::ready(Err(SourceError::Unavailable(
            Unavailable::Internal,
        ))))
    }
}
