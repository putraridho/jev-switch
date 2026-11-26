//! Antigravity's quota through `agy -p /usage` (DESIGN 6.5): a short-lived run that spends
//! no tokens. `proc` switches `agy`'s background updater off for it.

use crate::proc::{Flow, ProcRunner, ProcSpec, Program, Stderr};
use jev_usage_core::meter::Reading;
use jev_usage_core::port::{
    Cost, FetchFuture, SourceDescriptor, SourceError, SourceId, Trigger, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::Unavailable;
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

const DEADLINE: Duration = Duration::from_secs(45);

/// Reads Antigravity's windows from one `/usage` call.
pub struct AntigravityUsageSource {
    runner: Arc<ProcRunner>,
    clock: Arc<dyn Clock>,
}

impl AntigravityUsageSource {
    /// A source that starts `agy` through `runner` when it is installed.
    pub fn new(runner: Arc<ProcRunner>, clock: Arc<dyn Clock>) -> Self {
        Self { runner, clock }
    }
}

impl UsageSource for AntigravityUsageSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId::AntigravityCli,
            provider: ProviderKey::Antigravity,
            // Sparing: every run writes `agy`'s own logs and session files.
            trigger: Trigger::Interval {
                visible: Duration::from_mins(15),
                hidden: Duration::from_hours(1),
            },
            cost: Cost::Free,
            timeout: DEADLINE,
            // 2.5 x the hidden interval (DESIGN 7.7).
            fresh_for: Duration::from_mins(150),
            needs_login: false,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(async move {
            let spec = ProcSpec {
                program: Program::Agy,
                args: &["-p", "/usage", "--output-format", "json"],
                stdin: b"",
                deadline: DEADLINE,
                stderr: Stderr::Discard,
                max_output: 256 * 1024,
            };
            let mut output = Zeroizing::new(Vec::new());
            let exit = self
                .runner
                .run(&spec, &mut |line| {
                    output.extend_from_slice(line);
                    Flow::Continue
                })
                .await
                .map_err(SourceError::Unavailable)?;
            if !exit.success {
                return Err(SourceError::Unavailable(Unavailable::ServerError));
            }
            let observed_at = self.clock.now();
            let meters =
                jev_usage_core::parse::antigravity::parse_usage(&output, observed_at.seconds())
                    .map_err(SourceError::Unavailable)?;
            Ok(Reading {
                meters: meters.without_implausible_resets(observed_at),
                plan: None,
                observed_at,
            })
        })
    }
}
