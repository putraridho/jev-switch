//! How far up the ladder a provider is proven, and what a failed attempt says about the rung
//! it fell off. The rungs, bottom to top: installed, authenticated, entitled,
//! protocol-compatible, validated for this project. Each verified rung carries its evidence
//! and when it was seen; a rung nothing has shown stays unverified, never assumed fine.
//!
//! A failure that says something lasting (a login that failed, a retired client, a folder the
//! CLI refuses) holds the provider until the underlying fact changes (see [`ReadinessKey`]) or
//! a long expiry passes. A transient one (an empty or stalled answer) is left to the circuit
//! breaker (`domain::breaker`), which pauses briefly and lets one trial attempt through.

use std::collections::BTreeMap;

use jev_backend::{ErrorKind, ProviderId};

use super::availability::{Availability, State};
use super::login_hints::{install_hint, login_instruction};

/// How long a lasting failure holds a provider when nothing it depends on changes.
pub const HOLD_MS: u64 = 24 * 60 * 60 * 1000;
/// How long a handshake answer is trusted before the cheap check runs again.
pub const HANDSHAKE_TTL_MS: u64 = 6 * 60 * 60 * 1000;
/// Evidence younger than this is not rewritten, so a busy session does not save on every turn.
pub const EVIDENCE_REFRESH_MS: u64 = 60 * 60 * 1000;
/// Records kept; the least recently changed go first.
const MAX_RECORDS: usize = 64;

/// One step of the ladder. A rung is reached only when every rung below it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rung {
    Installed,
    Authenticated,
    Entitled,
    ProtocolCompatible,
    ValidatedForProject,
}

impl Rung {
    pub const ALL: [Rung; 5] = [Self::Installed, Self::Authenticated, Self::Entitled, Self::ProtocolCompatible, Self::ValidatedForProject];

    /// The stable name saved to disk.
    pub fn id(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Authenticated => "authenticated",
            Self::Entitled => "entitled",
            Self::ProtocolCompatible => "protocol",
            Self::ValidatedForProject => "project",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Authenticated => "logged in",
            Self::Entitled => "entitled",
            Self::ProtocolCompatible => "protocol-compatible",
            Self::ValidatedForProject => "validated for this project",
        }
    }
}

/// The rung a provider's adapter needs before routing may use it. Installed and logged in is
/// what a probe can prove without spending quota; the rungs above it are shown as unverified
/// until a handshake or an answered turn proves them, and only a rung that has *failed*
/// keeps a provider out.
pub fn needed_rung(_provider: ProviderId) -> Rung {
    Rung::Authenticated
}

/// What a rung was verified by, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub what: String,
    pub at_ms: u64,
}

// ------------------------------------------------------------------- failure classes

/// What to do about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// It will fail the same way until a fact changes (CLI, account, backend, project trust),
    /// or [`HOLD_MS`] pass.
    UntilChanged,
    /// It may pass by itself: a short expiring pause with a trial attempt (`domain::breaker`).
    Breaker,
}

/// What a refusal covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Provider,
    /// Only the model that was asked for (the account lacks it, others may work).
    Model,
}

/// Which rung a failure says is missing, and how to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureClass {
    pub rung: Rung,
    pub policy: RetryPolicy,
    pub scope: Scope,
}

/// The failure class of an attempt's error; `None` when it says nothing about readiness (a
/// usage limit has its own cooldown and proves the rungs below it; unknown errors prove nothing).
pub fn classify(kind: &ErrorKind) -> Option<FailureClass> {
    let (rung, policy, scope) = match kind {
        ErrorKind::Auth => (Rung::Authenticated, RetryPolicy::UntilChanged, Scope::Provider),
        ErrorKind::NotEntitled => (Rung::Entitled, RetryPolicy::UntilChanged, Scope::Model),
        ErrorKind::Unsupported => (Rung::ProtocolCompatible, RetryPolicy::UntilChanged, Scope::Provider),
        ErrorKind::UnusableHere => (Rung::ValidatedForProject, RetryPolicy::UntilChanged, Scope::Provider),
        ErrorKind::Empty | ErrorKind::Stalled => (Rung::ProtocolCompatible, RetryPolicy::Breaker, Scope::Provider),
        ErrorKind::Quota { .. } | ErrorKind::Other => return None,
    };
    Some(FailureClass { rung, policy, scope })
}

// ------------------------------------------------------------------- the saved facts

/// What a saved verdict depends on: when any of it changes, the verdict is void. Only
/// stamps and labels, never a credential or a prompt. An empty part means "not known".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadinessKey {
    /// The CLI: its path and file stamp (a new version is a new file).
    pub cli: String,
    /// The effective backend that pays (`Funding::label`).
    pub backend: String,
    /// The account: a stamp of the login file (its time and size), not its content.
    pub account: String,
}

/// A lasting failure that holds a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub rung: Rung,
    pub what: String,
    pub at_ms: u64,
    /// The hold expires at this time (epoch ms) if nothing changed before.
    pub until_ms: u64,
}

/// What is known about one provider in one project under one [`ReadinessKey`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub key: ReadinessKey,
    pub evidence: BTreeMap<Rung, Evidence>,
    pub failed: Option<Failed>,
    /// Models the account refused (not on its plan).
    pub refused: Vec<String>,
    /// When this record last changed (epoch ms): the newer copy wins a merge.
    pub updated_ms: u64,
}

impl Record {
    /// Whether the protocol check ran recently enough (passed or failed) not to run again.
    pub fn handshake_fresh(&self, now_ms: u64) -> bool {
        let recent = |at_ms: u64| now_ms.saturating_sub(at_ms) < HANDSHAKE_TTL_MS;
        self.evidence.get(&Rung::ProtocolCompatible).is_some_and(|e| recent(e.at_ms))
            || self.failed.as_ref().is_some_and(|f| f.rung == Rung::ProtocolCompatible && recent(f.at_ms))
    }
}

/// The saved records, per (provider, project key).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readiness {
    records: BTreeMap<(ProviderId, String), Record>,
}

impl Readiness {
    /// The record under `key`; a record saved under another key is void and not seen.
    pub fn record(&self, p: ProviderId, project: &str, key: &ReadinessKey) -> Option<&Record> {
        self.records.get(&(p, project.to_string())).filter(|r| &r.key == key)
    }

    fn slot(&mut self, p: ProviderId, project: &str, key: &ReadinessKey) -> &mut Record {
        let r = self.records.entry((p, project.to_string())).or_default();
        if &r.key != key {
            *r = Record { key: key.clone(), ..Record::default() };
        }
        r
    }

    /// Record that `rung` was seen working. Returns whether anything changed (fresh evidence
    /// is not rewritten). A failure of that rung ends.
    pub fn verify(&mut self, p: ProviderId, project: &str, key: &ReadinessKey, rung: Rung, what: &str, now_ms: u64) -> bool {
        let r = self.slot(p, project, key);
        let fresh = r.evidence.get(&rung).is_some_and(|e| now_ms.saturating_sub(e.at_ms) < EVIDENCE_REFRESH_MS && e.what == what);
        let failed_here = r.failed.as_ref().is_some_and(|f| f.rung == rung);
        if fresh && !failed_here {
            return false;
        }
        r.evidence.insert(rung, Evidence { what: what.to_string(), at_ms: now_ms });
        if failed_here {
            r.failed = None;
        }
        r.updated_ms = now_ms;
        true
    }

    /// An answered turn proves every rung up to `rung` and ends any hold.
    pub fn verify_through(&mut self, p: ProviderId, project: &str, key: &ReadinessKey, rung: Rung, what: &str, now_ms: u64) -> bool {
        let mut changed = false;
        for r in Rung::ALL.into_iter().filter(|r| *r <= rung) {
            changed |= self.verify(p, project, key, r, what, now_ms);
        }
        let r = self.slot(p, project, key);
        if r.failed.take().is_some() {
            r.updated_ms = now_ms;
            changed = true;
        }
        changed
    }

    /// Hold the provider after a lasting failure at the class's rung; the rungs above it are
    /// no longer proven. Does nothing for a transient or model-scoped class.
    pub fn fail(&mut self, p: ProviderId, project: &str, key: &ReadinessKey, class: FailureClass, what: &str, now_ms: u64) -> bool {
        if class.policy != RetryPolicy::UntilChanged || class.scope != Scope::Provider {
            return false;
        }
        let r = self.slot(p, project, key);
        r.evidence.retain(|rung, _| *rung < class.rung);
        r.failed = Some(Failed { rung: class.rung, what: what.to_string(), at_ms: now_ms, until_ms: now_ms.saturating_add(HOLD_MS) });
        r.updated_ms = now_ms;
        self.trim();
        true
    }

    pub fn refuse_model(&mut self, p: ProviderId, project: &str, key: &ReadinessKey, model: &str, now_ms: u64) {
        let r = self.slot(p, project, key);
        if !r.refused.iter().any(|m| m == model) {
            r.refused.push(model.to_string());
            r.updated_ms = now_ms;
        }
    }

    /// The hold on `p` in this project, if one is still in force.
    pub fn holding(&self, p: ProviderId, project: &str, key: &ReadinessKey, now_ms: u64) -> Option<&Failed> {
        self.record(p, project, key)?.failed.as_ref().filter(|f| f.until_ms > now_ms)
    }

    /// The user acted on a rung (logged in again, started a new session in a fixed folder): the
    /// holds of `p` at that rung end, in every project.
    pub fn forgive(&mut self, p: ProviderId, rung: Rung, now_ms: u64) -> bool {
        let mut changed = false;
        for ((provider, _), r) in &mut self.records {
            if *provider == p && r.failed.as_ref().is_some_and(|f| f.rung == rung) {
                r.failed = None;
                r.updated_ms = now_ms;
                changed = true;
            }
        }
        changed
    }

    /// Per record, the copy that changed last.
    pub fn merged(&self, other: &Self) -> Self {
        let mut records = self.records.clone();
        for (k, theirs) in &other.records {
            if records.get(k).is_none_or(|mine| mine.updated_ms < theirs.updated_ms) {
                records.insert(k.clone(), theirs.clone());
            }
        }
        let mut merged = Self { records };
        merged.trim();
        merged
    }

    fn trim(&mut self) {
        while self.records.len() > MAX_RECORDS {
            let Some(oldest) = self.records.iter().min_by_key(|(_, r)| r.updated_ms).map(|(k, _)| k.clone()) else { return };
            self.records.remove(&oldest);
        }
    }

    pub fn entries(&self) -> &BTreeMap<(ProviderId, String), Record> {
        &self.records
    }
}

impl From<BTreeMap<(ProviderId, String), Record>> for Readiness {
    fn from(records: BTreeMap<(ProviderId, String), Record>) -> Self {
        Self { records }
    }
}

// ------------------------------------------------------------------------ the ladder

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RungState {
    Verified(Evidence),
    Failed(Failed),
    /// Nothing has shown it either way.
    Unverified,
}

/// One provider's rungs as of now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ladder {
    pub rows: Vec<(Rung, RungState)>,
}

impl Ladder {
    /// What the probe saw, then what saved evidence and failures add. `probed_ms` is when the
    /// probe answered.
    pub fn build(a: &Availability, probed_ms: u64, record: Option<&Record>, now_ms: u64) -> Self {
        let seen = |what: String| RungState::Verified(Evidence { what, at_ms: probed_ms });
        let missing = |what: &str| RungState::Failed(Failed { rung: Rung::Installed, what: what.to_string(), at_ms: probed_ms, until_ms: 0 });
        let saved = |rung: Rung| record.and_then(|r| r.evidence.get(&rung)).map_or(RungState::Unverified, |e| RungState::Verified(e.clone()));
        let mut rows: Vec<(Rung, RungState)> = Rung::ALL
            .into_iter()
            .map(|rung| {
                let state = match (rung, a.state) {
                    (Rung::Installed, State::NotInstalled) => missing(&a.detail),
                    (Rung::Installed, _) => seen("found".to_string()),
                    (Rung::Authenticated, State::Ready) => seen(a.detail.clone()),
                    (Rung::Authenticated, State::NotLoggedIn | State::NoKey) => RungState::Failed(Failed { rung, what: a.detail.clone(), at_ms: probed_ms, until_ms: 0 }),
                    (Rung::Authenticated, _) => saved(rung),
                    (Rung::Entitled, State::Ready) if a.plan.is_some() => seen(format!("plan: {}", a.plan.clone().unwrap_or_default())),
                    _ => saved(rung),
                };
                (rung, state)
            })
            .collect();
        if let Some(f) = record.and_then(|r| r.failed.as_ref()).filter(|f| f.until_ms > now_ms) {
            if let Some(row) = rows.iter_mut().find(|(rung, _)| *rung == f.rung) {
                row.1 = RungState::Failed(f.clone());
            }
        }
        // Nothing above a failed or unverified rung counts as reached, whatever was saved.
        Self { rows }
    }

    /// The highest rung with every rung up to it verified.
    pub fn reached(&self) -> Option<Rung> {
        self.rows.iter().take_while(|(_, s)| matches!(s, RungState::Verified(_))).map(|(r, _)| *r).last()
    }

    /// The lowest failed rung.
    pub fn failure(&self) -> Option<(Rung, &Failed)> {
        self.rows.iter().find_map(|(r, s)| match s {
            RungState::Failed(f) => Some((*r, f)),
            _ => None,
        })
    }

    /// Whether routing may use the provider: no rung has failed and the needed one is reached.
    pub fn ready_for(&self, needed: Rung) -> bool {
        self.failure().is_none() && self.reached().is_some_and(|r| r >= needed)
    }

    /// The newest verification time on the ladder.
    pub fn last_evidence_ms(&self) -> Option<u64> {
        self.rows.iter().filter_map(|(_, s)| if let RungState::Verified(e) = s { Some(e.at_ms) } else { None }).max()
    }
}

/// What to do about a rung that failed, for `provider`.
pub fn fix_steps(provider: ProviderId, rung: Rung) -> Vec<String> {
    match rung {
        Rung::Installed => vec![install_hint(provider).to_string()],
        Rung::Authenticated => vec![format!("Log in: {}", login_instruction(provider)), "Then /providers refresh".to_string()],
        Rung::Entitled => vec!["Pick a model your plan includes, or upgrade the plan".to_string()],
        Rung::ProtocolCompatible => {
            vec!["Update the CLI (or replace it, as for Gemini: use Antigravity `agy`)".to_string(), "Then /providers probe checks it again".to_string()]
        }
        Rung::ValidatedForProject => match provider {
            ProviderId::Codex => vec!["Work inside a git repository (git init)".to_string()],
            ProviderId::Gemini => vec!["Trust this folder: run `agy` (or `gemini`) here once and accept the trust prompt".to_string()],
            _ => vec!["Work in another folder, or start the CLI here once to see what it asks for".to_string()],
        },
    }
}

#[cfg(test)]
mod tests;
