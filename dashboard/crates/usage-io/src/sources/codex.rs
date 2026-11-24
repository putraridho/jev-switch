//! `codex app-server` as a source (DESIGN 6.3): a short-lived process that costs no quota.
//! The type and the descriptor are fixed here; T04 writes the body.

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

/// Reads Codex's windows and plan from one `account/rateLimits/read` call.
pub struct CodexAppServerSource {
    _runner: Arc<ProcRunner>,
    _clock: Arc<dyn Clock>,
    _facts: Arc<FactsCell>,
}

impl CodexAppServerSource {
    /// A source that starts `codex` through `runner` when the login facts say `Ready`.
    pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            _runner: runner,
            _clock: clock,
            _facts: facts,
        }
    }
}

impl UsageSource for CodexAppServerSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId::CodexAppServer,
            provider: ProviderKey::Codex,
            trigger: Trigger::Interval {
                visible: Duration::from_mins(5),
                hidden: Duration::from_mins(15),
            },
            cost: Cost::Free,
            timeout: Duration::from_secs(15),
            // 2.5 x the hidden interval (DESIGN 7.7): 37.5 min.
            fresh_for: Duration::from_secs(2250),
            needs_login: true,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(std::future::ready(Err(SourceError::Unavailable(
            Unavailable::Internal,
        ))))
    }
}
