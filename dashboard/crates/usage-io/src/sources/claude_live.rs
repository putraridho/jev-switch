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
    runner: Arc<ProcRunner>,
    clock: Arc<dyn Clock>,
    facts: Arc<FactsCell>,
}

impl ClaudeLiveSource {
    /// A source that starts `claude` through `runner` when the login facts say `Ready`.
    pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>, facts: Arc<FactsCell>) -> Self {
        Self {
            runner,
            clock,
            facts,
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
        Box::pin(async move {
            use crate::probe::Readiness;
            use crate::proc::{Flow, ProcSpec, Program, Stderr};
            let unavailable = match self.facts.get().readiness(ProviderKey::Claude) {
                Readiness::Ready => None,
                Readiness::NotInstalled => Some(Unavailable::NotInstalled),
                Readiness::NotLoggedIn => Some(Unavailable::NotLoggedIn),
                Readiness::Unknown => Some(Unavailable::Timeout),
            };
            if let Some(error) = unavailable {
                return Err(SourceError::Unavailable(error));
            }
            let spec = ProcSpec {
                program: Program::Claude,
                args: &[
                    "-p",
                    "--model",
                    "haiku",
                    "--tools",
                    "",
                    "--no-session-persistence",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "Hi",
                ],
                stdin: b"",
                deadline: Duration::from_mins(1),
                stderr: Stderr::Discard,
                max_output: 4 * 1024 * 1024,
            };
            let mut answer = None;
            let exit = self
                .runner
                .run(&spec, &mut |line| {
                    if let Some(meters) = jev_usage_core::parse::claude_event::parse_line(line) {
                        answer = Some(meters);
                    }
                    Flow::Continue
                })
                .await
                .map_err(SourceError::Unavailable)?;
            if !exit.success {
                return Err(SourceError::Unavailable(Unavailable::Protocol));
            }
            let meters = answer.ok_or(SourceError::Unavailable(Unavailable::Protocol))?;
            Ok(jev_usage_core::meter::Reading {
                meters,
                plan: self.facts.get().plan(ProviderKey::Claude),
                observed_at: self.clock.now(),
            })
        })
    }
}
