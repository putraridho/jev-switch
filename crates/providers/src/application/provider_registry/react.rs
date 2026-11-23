//! What a failed attempt teaches the router: limits, refused models, login and
//! directory blocks.

use std::collections::HashSet;

use jev_backend::{ErrorKind, ProviderId, TurnError};
use jev_catalog::domain::model_table::models;
use crate::domain::login_hints::{login_target, unsupported_hint, unusable_here_hint};

use super::{BlockReason, Runtime};

impl Runtime {
    /// Apply what a failed attempt teaches the router and say what happens next. `Some`
    /// is the note for "try the next model"; `None` means the failure would likely recur
    /// on every provider, so the caller stops. Shared by a turn's failover
    /// ([`jev_turn::application::engine::Engine`]) and by orchestration workers
    /// ([`jev_orchestration::application::delegator::Delegator`]) so a limit, a login or an entitlement is handled
    /// one way everywhere.
    pub fn react(
        &mut self,
        provider: ProviderId,
        model: &str,
        err: &TurnError,
        excluded: &mut HashSet<(ProviderId, String)>,
        now: u64,
    ) -> Option<String> {
        match &err.kind {
            ErrorKind::Quota { resets_at } => {
                self.cool_down(provider, resets_at.as_deref(), now);
                let when = resets_at.as_deref().map(|r| format!(" (resets {r})")).unwrap_or_default();
                Some(format!("{provider} hit its limit{when}: trying the next model"))
            }
            ErrorKind::NotEntitled => {
                self.mark_unusable(provider, model);
                Some(format!("{provider}:{model} is not on your plan: trying the next model"))
            }
            ErrorKind::Auth => {
                self.blocked.insert(provider, BlockReason::Auth);
                Some(format!(
                    "{provider} is not logged in or its login expired (log in to {} again, then /providers refresh): trying the next model",
                    login_target(provider)
                ))
            }
            ErrorKind::UnusableHere => {
                self.blocked.insert(provider, BlockReason::UnusableHere);
                Some(format!("{provider} {}: trying the next model", unusable_here_hint(provider)))
            }
            ErrorKind::Unsupported => {
                self.blocked.insert(provider, BlockReason::Unsupported);
                Some(format!("{provider} {}: trying the next model", unsupported_hint(provider)))
            }
            ErrorKind::Stalled | ErrorKind::Empty => {
                excluded.insert((provider, model.to_string()));
                let what = if err.kind == ErrorKind::Stalled { "stopped answering" } else { "gave an empty answer" };
                if excluded.iter().filter(|(failed_provider, _)| *failed_provider == provider).count() >= 2 {
                    excluded.extend(
                        models()
                            .iter()
                            .filter(|candidate| candidate.provider == provider)
                            .map(|candidate| (candidate.provider, candidate.id.to_string())),
                    );
                    return Some(format!("{provider} {what} on {model} too: skipping {provider} for this turn"));
                }
                Some(format!("{provider}:{model} {what}: trying the next model"))
            }
            ErrorKind::Other => None,
        }
    }
}
