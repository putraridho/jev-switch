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

use jev_backend::{PlanUsage, ProviderId};
use crate::domain::availability::Availability;
use crate::domain::breaker::Breakers;
use crate::domain::funding::Fundings;
use crate::domain::handshake::Handshake;
use crate::domain::readiness::{Readiness, ReadinessKey};
use crate::domain::reset_time::cooldowns::Cooldowns;

/// A boxed future the port hands back, so the trait stays object safe.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the machine can tell the readiness ladder: the stamps a saved verdict depends on, the
/// saved verdicts, and the capability handshake. None of it sends a prompt.
pub trait ReadinessHost: Debug + Send + Sync {
    /// The CLI and the account of `provider` (the backend part is left for the caller).
    fn readiness_key(&self, provider: ProviderId) -> ReadinessKey;
    /// The folder part of a verdict's key: the path and what the CLI's folder rule says now.
    fn project_key(&self, provider: ProviderId, dir: &Path) -> String;
    /// The cheap capability check; it spends no quota.
    fn handshake(&self, provider: ProviderId, dir: PathBuf) -> BoxFuture<Handshake>;
    fn load_readiness(&self, path: &Path) -> Readiness;
    /// Merge `readiness` into the saved records, write them, and return the merged set.
    fn save_readiness(&self, path: &Path, readiness: &Readiness) -> std::io::Result<Readiness>;
}

/// What the machine can tell the provider registry.
pub trait ProviderHost: ReadinessHost {
    /// One provider's probe, with no time cap of its own: gathers the facts (PATH, status
    /// command, files, environment) and lets the pure decision answer.
    fn probe(&self, provider: ProviderId) -> BoxFuture<Availability>;
    /// A saved probe answer that is still fresh and was made under `fingerprint`.
    fn load_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64) -> Option<Vec<Availability>>;
    fn save_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64, avail: &[Availability]);
    fn load_cooldowns(&self, path: &Path) -> Cooldowns;
    fn save_cooldowns(&self, path: &Path, cooldowns: &Cooldowns, now_ms: u64) -> std::io::Result<Cooldowns>;
    /// The saved circuit breakers (see [`crate::domain::breaker`]); nothing when there is no file.
    fn load_breakers(&self, path: &Path) -> Breakers;
    /// Merge `breakers` into the saved ones, write them, and return the merged set.
    fn save_breakers(&self, path: &Path, breakers: &Breakers) -> std::io::Result<Breakers>;
    /// Context windows the machine's own backends report, filed under the backend that
    /// reported them; empty when none is known.
    fn reported_windows(&self) -> crate::domain::window_book::WindowBook;
    fn cooldown_stamp(&self, path: &Path) -> Option<(std::time::SystemTime, u64)>;
    fn resolve_reset_time(&self, text: &str, zone: Option<&str>) -> Option<u64>;
    fn reset_zone_exists(&self, zone: &str) -> bool;
    /// Where plan readings are saved; `None` without a state directory.
    fn plan_path(&self) -> Option<PathBuf>;
    fn load_plans(&self, path: &Path) -> HashMap<ProviderId, PlanUsage>;
    fn save_plans(&self, path: &Path, plans: &HashMap<ProviderId, PlanUsage>);
    /// Whether the reading's window has not reset by `now_s`.
    fn plan_is_current(&self, plan: &PlanUsage, now_s: u64) -> bool;
    /// Codex's own latest reading, from its session files.
    fn latest_codex_plan(&self) -> Option<PlanUsage>;
    /// Antigravity's latest quota reading for Gemini, asked from `agy` itself; `None` when
    /// Gemini does not run through `agy`, or on any failure.
    fn fetch_gemini_plan(&self) -> BoxFuture<Option<PlanUsage>> {
        Box::pin(std::future::ready(None))
    }
    /// DeepSeek's remaining balance for display (`$12.34 left`); `None` on any failure.
    fn fetch_credit(&self, key: String) -> BoxFuture<Option<String>>;
    /// A setting (`JEV_ALLOW_CREDITS`, `DEEPSEEK_API_KEY`): the environment first, then the key file.
    fn env(&self, name: &str) -> Option<String>;
    /// Unix time in milliseconds.
    fn now_ms(&self) -> u64;
    /// The directory holding logs, sessions and caches; `None` without one.
    fn state_dir(&self) -> Option<PathBuf>;
    /// What funds each provider's CLI on this machine, from its configuration and credentials
    /// (presence and kind only). Reads small files; never runs a CLI.
    fn fundings(&self) -> Fundings;
}

/// What the funding probes may ask the machine. Answers are presence and file text, never
/// a credential's value: a key is asked about with `*_set`, so it cannot be read, logged
/// or leak into a label.
pub trait AccountFacts: Debug + Send + Sync {
    /// A setting is set and non-empty in the environment or `~/.jev-router.env`.
    fn setting_set(&self, name: &str) -> bool;
    /// A variable is set and non-empty in this process's own environment (what a child
    /// CLI inherits; the key file is not).
    fn process_env_set(&self, name: &str) -> bool;
    fn home_dir(&self) -> Option<PathBuf>;
    /// Codex's folder: `$CODEX_HOME`, else `~/.codex`.
    fn codex_home(&self) -> Option<PathBuf>;
    fn read_text(&self, path: &Path) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
}
