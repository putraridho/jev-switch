//! The doors the provider use cases have to the machine: the provider probes, the probe cache,
//! the saved plan readings, Codex's own plan reading, DeepSeek's balance, and the settings,
//! clock and state directory they need.
//!
//! The adapter is `providers::infrastructure::probe_facts::SystemProbes`; the composition
//! roots (the binaries and the TUI start) hand it to [`Runtime`](super::provider_registry::Runtime).

use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::backend::{PlanUsage, ProviderId};
use crate::providers::domain::availability::Availability;

/// A boxed future the port hands back, so the trait stays object safe.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the machine can tell the provider registry.
pub trait ProviderHost: Debug + Send + Sync {
    /// One provider's probe, with no time cap of its own: gathers the facts (PATH, status
    /// command, files, environment) and lets the pure decision answer.
    fn probe(&self, provider: ProviderId) -> BoxFuture<Availability>;
    /// A saved probe answer that is still fresh and was made under `fingerprint`.
    fn load_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64) -> Option<Vec<Availability>>;
    fn save_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64, avail: &[Availability]);
    /// Where plan readings are saved; `None` without a state directory.
    fn plan_path(&self) -> Option<PathBuf>;
    fn load_plans(&self, path: &Path) -> HashMap<ProviderId, PlanUsage>;
    fn save_plans(&self, path: &Path, plans: &HashMap<ProviderId, PlanUsage>);
    /// Whether the reading's window has not reset by `now_s`.
    fn plan_is_current(&self, plan: &PlanUsage, now_s: u64) -> bool;
    /// Codex's own latest reading, from its session files.
    fn latest_codex_plan(&self) -> Option<PlanUsage>;
    /// DeepSeek's remaining balance for display (`$12.34 left`); `None` on any failure.
    fn fetch_credit(&self, key: String) -> BoxFuture<Option<String>>;
    /// A setting (`JEV_ALLOW_CREDITS`, `DEEPSEEK_API_KEY`): the environment first, then the key file.
    fn env(&self, name: &str) -> Option<String>;
    /// Unix time in milliseconds.
    fn now_ms(&self) -> u64;
    /// The directory holding logs, sessions and caches; `None` without one.
    fn state_dir(&self) -> Option<PathBuf>;
}
