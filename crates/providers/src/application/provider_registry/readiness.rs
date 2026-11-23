//! The readiness ladder on the runtime: what each provider has proven (installed, logged in,
//! entitled, protocol-compatible, validated for this project), what holds it back, and the
//! capability handshake that proves the cheap rungs without a prompt.

use std::path::Path;

use jev_backend::{ErrorKind, ProviderId};
use jev_shared::domain::duration_text::format_left;
use jev_shared::ALL_PROVIDERS;

use crate::domain::availability::State;
use crate::domain::breaker::Admission;
use crate::domain::handshake::{Check, Handshake};
use crate::domain::readiness::{classify, needed_rung, Failed, FailureClass, Ladder, ReadinessKey, RetryPolicy, Rung, Scope};
use crate::domain::readiness_view::{describe, ReadinessView};

use super::Runtime;

/// `<state_dir>/provider-readiness.json`.
const FILE: &str = "provider-readiness.json";

/// What a failure kind says went wrong, for the saved hold.
fn failure_text(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Auth => "the login failed or expired",
        ErrorKind::Unsupported => "the service no longer accepts this CLI (retired client)",
        ErrorKind::UnusableHere => "the CLI refuses this folder (untrusted, or not a git repository)",
        ErrorKind::NotEntitled => "the model is not on the plan",
        _ => "failed",
    }
}

fn check_text(c: &Check) -> String {
    match c {
        Check::Passed(w) => format!("ok ({w})"),
        Check::Failed(w) => format!("FAILED ({w})"),
        Check::Unverified(w) => format!("unverified ({w})"),
    }
}

impl Runtime {
    /// Tell the runtime which project this session works in: the saved verdicts of this
    /// project are read, and models the account already refused here are not offered again.
    pub fn attach_project(&mut self, dir: &Path) {
        self.project_dir = Some(dir.to_path_buf());
        self.readiness_path = self.host.state_dir().map(|d| d.join(FILE));
        if let Some(path) = &self.readiness_path {
            self.readiness = self.host.load_readiness(path);
        }
        self.rekey();
        for p in ALL_PROVIDERS {
            let (project, key) = self.scope(p);
            let refused: Vec<String> = self.readiness.record(p, &project, &key).map(|r| r.refused.clone()).unwrap_or_default();
            self.unusable_models.extend(refused.into_iter().map(|m| (p, m)));
        }
    }

    /// Recompute what the saved verdicts depend on: a new CLI file, login file, funding or
    /// folder trust makes the old verdicts void.
    pub(super) fn rekey(&mut self) {
        let Some(dir) = self.project_dir.clone() else { return };
        for p in ALL_PROVIDERS {
            let mut key = self.host.readiness_key(p);
            key.backend = self.fundings.identity(p);
            self.keys.insert(p, key);
            self.project_keys.insert(p, self.host.project_key(p, &dir));
        }
    }

    pub(super) fn scope(&self, p: ProviderId) -> (String, ReadinessKey) {
        (self.project_keys.get(&p).cloned().unwrap_or_default(), self.keys.get(&p).cloned().unwrap_or_default())
    }

    /// The lasting failure that holds `p` in this project, if one is in force.
    pub(super) fn held(&self, p: ProviderId, now_ms: u64) -> Option<Failed> {
        let (project, key) = self.scope(p);
        self.readiness.holding(p, &project, &key, now_ms).cloned()
    }

    /// `p`'s ladder as of now; `None` for a provider that was not probed.
    pub fn ladder(&self, p: ProviderId, now_ms: u64) -> Option<Ladder> {
        let a = self.avail.iter().find(|a| a.provider == p)?;
        let (project, key) = self.scope(p);
        Some(Ladder::build(a, self.probed_ms, self.readiness.record(p, &project, &key), now_ms))
    }

    /// Why routing leaves `p` out, in words that name the rung; `None` when it may be used.
    /// A pinned provider gets this text as its error.
    pub fn exclusion(&self, p: ProviderId, now_ms: u64) -> Option<String> {
        if let Some(f) = self.held(p, now_ms) {
            let first_fix = crate::domain::readiness::fix_steps(p, f.rung).into_iter().next().unwrap_or_default();
            return Some(format!(
                "held at the {} rung: {} (until the CLI, login, backend or folder changes, or {}). {first_fix}",
                f.rung.label(),
                f.what,
                format_left(f.until_ms.saturating_sub(now_ms))
            ));
        }
        match self.breakers.admission(p, now_ms) {
            Admission::Open { until_ms } => {
                return Some(format!("paused by its circuit breaker after repeated failures (one trial attempt in {})", format_left(until_ms.saturating_sub(now_ms))))
            }
            Admission::Trial { .. } => return Some("being tried after a pause; wait for that attempt".to_string()),
            Admission::Closed | Admission::HalfOpen => {}
        }
        let ladder = self.ladder(p, now_ms)?;
        (!ladder.ready_for(needed_rung(p))).then(|| format!("below the {} rung it needs", needed_rung(p).label()))
    }

    /// One view per provider that is ready by its probe: the rung reached, when it was last
    /// proven, when it is tried again, and what to do about a failed rung.
    pub fn readiness_views(&self, now_ms: u64) -> Vec<ReadinessView> {
        self.avail
            .iter()
            .filter(|a| a.state == State::Ready)
            .filter_map(|a| Some(describe(a.provider, &self.ladder(a.provider, now_ms)?, self.breakers.admission(a.provider, now_ms), now_ms)))
            .collect()
    }

    pub(super) fn readiness_lines(&self, now_ms: u64) -> Vec<String> {
        self.readiness_views(now_ms).into_iter().map(|v| format!("readiness: {}", v.line)).collect()
    }

    /// What a failed attempt says about the ladder: a lasting failure holds the provider (until
    /// a fact changes), a model refusal is remembered for the project, a transient one is the
    /// breaker's business.
    pub(super) fn note_readiness_failure(&mut self, p: ProviderId, model: &str, kind: &ErrorKind, now_ms: u64) {
        let Some(class) = classify(kind) else { return };
        let (project, key) = self.scope(p);
        let changed = match class.scope {
            Scope::Provider => self.readiness.fail(p, &project, &key, class, failure_text(kind), now_ms),
            Scope::Model => {
                self.readiness.refuse_model(p, &project, &key, model, now_ms);
                true
            }
        };
        if changed {
            self.save_readiness();
        }
    }

    pub(super) fn note_readiness_success(&mut self, p: ProviderId, now_ms: u64) {
        let (project, key) = self.scope(p);
        if self.readiness.verify_through(p, &project, &key, Rung::ValidatedForProject, "answered a turn", now_ms) {
            self.save_readiness();
        }
    }

    /// `/providers refresh` saw a login again: holds on the login end.
    pub(super) fn forgive_logins(&mut self, now_ms: u64) {
        let ready: Vec<ProviderId> = self.avail.iter().filter(|a| a.state == State::Ready).map(|a| a.provider).collect();
        let mut changed = false;
        for p in ready {
            changed |= self.readiness.forgive(p, Rung::Authenticated, now_ms);
        }
        if changed {
            self.save_readiness();
        }
    }

    /// `/new`: the folder may have been fixed (trusted, made a repository), so folder holds end
    /// and the folder's key is read again.
    pub(super) fn forgive_folders(&mut self, now_ms: u64) {
        self.rekey();
        let mut changed = false;
        for p in ALL_PROVIDERS {
            changed |= self.readiness.forgive(p, Rung::ValidatedForProject, now_ms);
        }
        if changed {
            self.save_readiness();
        }
    }

    /// Save, merging with what other processes saved; a failed save costs only the persistence.
    fn save_readiness(&mut self) {
        let Some(path) = &self.readiness_path else { return };
        match self.host.save_readiness(path, &self.readiness) {
            Ok(merged) => self.readiness = merged,
            Err(e) => eprintln!("could not save provider readiness to {}: {e}", path.display()),
        }
    }

    /// The capability handshake for every provider that is ready by its probe: proves the
    /// protocol and the folder without a prompt, so it spends no quota. A recent answer under
    /// the same key is reused unless `force`. Returns one line per provider it ran.
    pub async fn handshake(&mut self, force: bool) -> Vec<String> {
        let Some(dir) = self.project_dir.clone() else { return Vec::new() };
        let now = self.host.now_ms();
        let ready: Vec<ProviderId> = self.avail.iter().filter(|a| a.state == State::Ready).map(|a| a.provider).collect();
        let mut lines = Vec::new();
        for p in ready {
            let (project, key) = self.scope(p);
            if !force && self.readiness.record(p, &project, &key).is_some_and(|r| r.handshake_fresh(now)) {
                continue;
            }
            let found = self.host.handshake(p, dir.clone()).await;
            lines.push(format!("{p}: protocol {}; folder {}", check_text(&found.protocol), check_text(&found.project)));
            self.apply_handshake(p, &found, now);
        }
        self.save_readiness();
        lines
    }

    pub(super) fn apply_handshake(&mut self, p: ProviderId, found: &Handshake, now_ms: u64) {
        let (project, key) = self.scope(p);
        for (rung, check) in [(Rung::ProtocolCompatible, &found.protocol), (Rung::ValidatedForProject, &found.project)] {
            match check {
                Check::Passed(what) => {
                    self.readiness.verify(p, &project, &key, rung, what, now_ms);
                }
                Check::Failed(what) => {
                    let class = FailureClass { rung, policy: RetryPolicy::UntilChanged, scope: Scope::Provider };
                    self.readiness.fail(p, &project, &key, class, what, now_ms);
                }
                Check::Unverified(_) => {}
            }
        }
    }
}
