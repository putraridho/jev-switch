//! What the router has learned about providers while it runs (who is logged in, who
//! hit a limit, which models were refused), kept in [`Runtime`].
//!
//! The struct and its constructors live here; its behaviour is split by concern into
//! child modules, so the fields stay private to this subtree: [`probing`] (start, refresh,
//! late probe answers), [`usable`] (what routing may pick, cooldowns), [`react`] (what a
//! failed attempt teaches), [`plan`] (plan utilization, DeepSeek balance) and [`render`]
//! (the `/providers` text). The pure rules are in `providers/domain`
//! ([`usability`](crate::domain::usability), [`login_hints`](crate::domain::login_hints)).

mod plan;
pub mod disk_watch;
mod probing;
mod react;
mod render;
#[cfg(test)]
mod tests;
mod usable;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use jev_backend::{PlanUsage, ProviderId};
use jev_catalog::domain::model_table::{Allowances, Overrides};
use crate::application::ports::ProviderHost;
use crate::domain::availability::Availability;
use crate::domain::reset_time::cooldowns::Cooldowns;
use disk_watch::DiskWatch;

/// What the router knows about providers right now. Availability comes from probes;
/// cooldowns persist across runs; unusable models and plan utilization live only for
/// this run.
#[derive(Debug)]
pub struct Runtime {
    pub avail: Vec<Availability>,
    pub cooldowns: Cooldowns,
    pub allow: Allowances,
    pub overrides: Overrides,
    /// Where cooldowns are saved; `None` when there is no state directory.
    pub cooldown_path: Option<PathBuf>,
    /// The last copy of the cooldown file read, so limits other processes saved
    /// (`jev-code`, `delegate_mcp`) are seen without a restart. Behind a mutex because
    /// [`Runtime::usable`] only has `&self`.
    disk: Mutex<DiskWatch>,
    /// Models a provider refused at run time (e.g. not entitled), so they are not retried.
    pub unusable_models: HashSet<(ProviderId, String)>,
    /// The latest plan utilization each provider reported.
    pub plan: HashMap<ProviderId, PlanUsage>,
    /// Where `plan` is saved after every new reading; `None` in tests and without a state dir.
    plan_path: Option<PathBuf>,
    /// DeepSeek's remaining account balance, already formatted for display, filled in by a background fetch (it is pay per
    /// token, so a balance is what "credit left" means there, not a plan percentage).
    balance: std::sync::Arc<Mutex<Option<String>>>,
    /// Providers that failed in a way another provider can cover and that is not a usage
    /// limit, with why. Session-only and kept out of the persisted cooldowns on purpose:
    /// a login problem or an unusable directory is not a limit, and an hour-long cooldown
    /// would only outlast a re-login. See [`BlockReason`] for what clears each.
    pub blocked: HashMap<ProviderId, BlockReason>,
    /// The background retry of probes that timed out at startup, until
    /// [`Runtime::absorb_late`] has folded its answers into `avail`.
    late: Option<LateProbes>,
    /// The machine: probes, probe cache, saved plan readings, Codex's plan, DeepSeek's balance.
    host: Arc<dyn ProviderHost>,
}

/// Answers from the background retry, and what to tell the user about the ones applied.
struct LateProbes {
    answers: tokio::sync::mpsc::UnboundedReceiver<Availability>,
    /// What an answer may change: only a provider still Unknown (the slow-probe retry), or
    /// any provider whose answer differs from the cached one the start used.
    merge: LateMerge,
    /// Called from `absorb_late`, only for a provider that really became Ready.
    on_ready: Box<dyn Fn(&Availability) + Send + Sync>,
}

/// How a background answer is folded into the availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LateMerge {
    /// [`availability::merge_late`].
    FillUnknown,
    /// [`availability::merge_refresh`].
    Refresh,
}

/// The cache a start may read, and keep up to date.
#[derive(Debug, Clone)]
pub struct ProbeCache {
    pub path: PathBuf,
    pub fingerprint: u64,
}

impl fmt::Debug for LateProbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LateProbes")
    }
}

/// Why a provider is skipped for the rest of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The login failed. `/providers refresh` clears it once the probe sees a login.
    Auth,
    /// The CLI refuses to run in this directory (Codex outside a git repo, Gemini in an
    /// untrusted workspace). Probing cannot change that, so only `/new` or another
    /// project directory (a new session) clears it.
    UnusableHere,
    /// The service no longer accepts this CLI at all (Gemini CLI migrated to Antigravity).
    /// Neither a probe nor `/new` can change that; only an updated or replaced CLI, so it
    /// lasts the session.
    Unsupported,
}

impl Runtime {
    /// The one place a `Runtime` is assembled, so `start` and the tests share it.
    fn build(
        avail: Vec<Availability>,
        cooldowns: Cooldowns,
        allow: Allowances,
        overrides: Overrides,
        cooldown_path: Option<PathBuf>,
        host: Arc<dyn ProviderHost>,
    ) -> Self {
        Self {
            avail,
            cooldowns,
            allow,
            overrides,
            cooldown_path,
            disk: Mutex::new(DiskWatch::default()),
            unusable_models: HashSet::new(),
            plan: HashMap::new(),
            plan_path: None,
            balance: std::sync::Arc::default(),
            blocked: HashMap::new(),
            late: None,
            host,
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn for_tests(avail: Vec<Availability>) -> Self {
        let allow = crate::domain::usability::allowances_for(None, &avail);
        Self::build(avail, Cooldowns::default(), allow, Overrides::default(), None, Arc::new(crate::infrastructure::probe_facts::SystemProbes))
    }
}
