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
    runner: Arc<ProcRunner>,
    clock: Arc<dyn Clock>,
    facts: Arc<FactsCell>,
}

impl CodexAppServerSource {
    /// A source that starts `codex` through `runner` when the login facts say `Ready`.
    pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            runner,
            clock,
            facts,
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
        Box::pin(async move {
            use crate::probe::Readiness;
            use crate::proc::{Flow, ProcSpec, Program, Stderr};
            let unavailable = match self.facts.get().readiness(ProviderKey::Codex) {
                Readiness::Ready => None,
                Readiness::NotInstalled => Some(Unavailable::NotInstalled),
                Readiness::NotLoggedIn => Some(Unavailable::NotLoggedIn),
                Readiness::Unknown => Some(Unavailable::Timeout),
            };
            if let Some(error) = unavailable {
                return Err(SourceError::Unavailable(error));
            }
            let spec = ProcSpec { program: Program::Codex, args: &["app-server"], stdin: b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"jev-dashboard\",\"version\":\"0.1.0\"}}}\n{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"account/rateLimits/read\",\"params\":{\"excludeResetCreditDetails\":true}}\n", deadline: Duration::from_secs(15), stderr: Stderr::Discard, max_output: 4 * 1024 * 1024 };
            let mut answer = None;
            let exit = self
                .runner
                .run(&spec, &mut |line| {
                    if let Some(parsed) =
                        jev_usage_core::parse::codex::parse_rate_limits_response(line, 2)
                    {
                        answer = Some(parsed);
                        Flow::Done
                    } else {
                        Flow::Continue
                    }
                })
                .await
                .map_err(SourceError::Unavailable)?;
            let limits = answer
                .unwrap_or(Err(Unavailable::Protocol))
                .map_err(SourceError::Unavailable)?;
            let _ = exit;
            let observed_at = self.clock.now();
            Ok(jev_usage_core::meter::Reading {
                meters: limits.meters.without_implausible_resets(observed_at),
                plan: limits.plan,
                observed_at,
            })
        })
    }
}
