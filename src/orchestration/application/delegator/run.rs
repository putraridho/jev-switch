//! A whole delegation: judge the brief, then run workers until one succeeds.

use std::collections::HashSet;

use serde_json::Value;

use crate::backend::ProviderId;
use crate::judgment::domain::want::ProviderWant;
use crate::orchestration::application::delegator::outcome::{AttemptOutcome, Finished};
use crate::orchestration::domain::brief_signals::BriefSignals;
use crate::orchestration::domain::delegation::{DelegateRequest, DelegateResult};
use crate::orchestration::domain::worker_prompt::worker_prompt;

use super::{lock, Delegator};

/// A worker that fails in a way another provider can fix is retried this many times in
/// all (so: once more), on the next worker pick.
pub const MAX_WORKER_ATTEMPTS: u32 = 2;

/// Jev's judgment of a brief, as the signals a worker pick reads.
fn signals_of(w: &ProviderWant, read_only: bool) -> BriefSignals {
    BriefSignals { task: w.task, security_p: w.security_p, edit_p: w.edit_p, read_only }
}

impl Delegator {
    /// Judge the brief, then run workers until one succeeds or the attempts are used up.
    /// Nothing reaches the main tree here: that is [`Delegator::settle`]'s.
    pub(super) async fn run(&self, req: DelegateRequest) -> Finished {
        // The brief is a prompt like any other: the privacy policy decides which providers may
        // have it, and Jev (a cloud service) is asked only when some cloud provider may.
        let repo = self.worktrees.repo();
        let outgoing = worker_prompt(&req.brief, &req.files, req.read_only);
        let verdict = self.guards.privacy.check(&repo, &outgoing, &[]);
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        for p in crate::shared::ALL_PROVIDERS.into_iter().filter(|p| !verdict.permits(*p)) {
            excluded.extend(crate::catalog::domain::model_table::models().iter().filter(|m| m.provider == p).map(|m| (m.provider, m.id.to_string())));
        }
        if crate::shared::ALL_PROVIDERS.into_iter().all(|p| !verdict.permits(p)) {
            return Finished::done(DelegateResult::failed(format!("not delegated: {}", verdict.refusal_line()), Vec::new()));
        }
        let want = if self.guards.may_leave(&repo, &outgoing, &[]) {
            (self.judge)(crate::turn::domain::privacy::prepare(&self.guards.privacy, &req.brief).prompt).await
        } else {
            None
        };
        let signals = want.as_ref().map_or_else(BriefSignals::unjudged, |w| signals_of(w, req.read_only));
        let jev = want.map_or(Value::Null, |w| w.signals);
        let mut retries: Vec<String> = Vec::new();
        for attempt in 1..=MAX_WORKER_ATTEMPTS {
            match self.attempt(&req, &signals, &jev, &excluded, attempt).await {
                Ok(finished) => return finished.with_retries(retries),
                Err(AttemptOutcome::Stop(message)) => return Finished::done(DelegateResult::failed(message, retries)),
                Err(AttemptOutcome::Failed(f)) => {
                    let react = lock(&self.runtime).react(f.spec.provider, f.spec.id, &f.err, &mut excluded, (self.now)());
                    match react {
                        Some(note) if attempt < MAX_WORKER_ATTEMPTS => retries.push(note),
                        _ => {
                            let mut failed = DelegateResult::failed(format!("{} {} failed: {}", f.spec.provider, f.spec.id, f.err.message), retries);
                            failed.worker_id = f.id;
                            failed.provider = Some(f.spec.provider);
                            failed.model = Some(f.spec.id.to_string());
                            return Finished::done(failed);
                        }
                    }
                }
            }
        }
        Finished::done(DelegateResult::failed("no worker succeeded".into(), retries))
    }
}
