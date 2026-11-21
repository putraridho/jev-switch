//! Which providers the user can actually use right now. Decided once at startup,
//! BEFORE the first turn, because finding out by running a turn is slow (a logged-out
//! `codex` retries for ~35 s before it fails) and a turn sent to a provider the user
//! cannot use is a wasted turn.
//!
//! Every provider has the same shape: a thin IO function gathers cheap facts (a PATH
//! search, a status command, a small file), and a pure `decide_*` function turns
//! those facts into an [`Availability`]. Only the pure half is unit-tested, with
//! injected inputs, so no test touches real home files, real CLIs or the network.
//!
//! No probe ever sends a prompt. `grok` is never executed at all: here it is a lazy
//! mise wrapper and even `--version` starts a long install.


use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::ProviderId;

pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod deepseek;
pub(crate) mod gemini;
pub(crate) mod grok;
pub(crate) mod host;
pub(crate) mod mise;
pub mod render;
pub mod retry;
#[cfg(test)]
mod tests;
pub mod usable;

/// Each probe gets this long; a slower one is reported as [`State::Unknown`] rather
/// than holding up startup.
pub const PROBE_CAP: Duration = Duration::from_secs(5);

/// Can the user use this provider?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ready,
    NotInstalled,
    NotLoggedIn,
    /// An API-key provider with no key configured.
    NoKey,
    /// The probe could not tell (timed out, crashed, unreadable output).
    Unknown,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInstalled => "not installed",
            Self::NotLoggedIn => "not logged in",
            Self::NoKey => "no key",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub provider: ProviderId,
    pub state: State,
    /// The account plan when the login says so ("max_5x", "plus", "pay-per-token").
    pub plan: Option<String>,
    /// Models this login may use, when the CLI lists them. `None` means "no list known".
    pub models: Option<Vec<String>>,
    /// Why, in a few words, for `/providers`.
    pub detail: String,
    /// False when the probe was written from documentation and has never been
    /// checked against the real CLI.
    pub verified: bool,
}

/// Whether a provider's probe has been run against the real CLI. One place, so a
/// probe is promoted to verified by changing one match arm.
fn is_verified(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex | ProviderId::DeepSeek)
}

fn avail(provider: ProviderId, state: State, detail: impl Into<String>) -> Availability {
    Availability { provider, state, plan: None, models: None, detail: detail.into(), verified: is_verified(provider) }
}

// ---------------------------------------------------------------- public API

/// Probe all five providers concurrently, each capped at [`PROBE_CAP`]. `probe` is the one
/// provider's uncapped probe: it gathers the facts, so it lives outside the domain.
pub async fn probe_all<P, Fut>(probe: P) -> Vec<Availability>
where
    P: Fn(ProviderId) -> Fut,
    Fut: Future<Output = Availability>,
{
    let (claude, deepseek, codex, gemini, grok) = tokio::join!(
        capped(PROBE_CAP, ProviderId::Claude, probe(ProviderId::Claude)),
        capped(PROBE_CAP, ProviderId::DeepSeek, probe(ProviderId::DeepSeek)),
        capped(PROBE_CAP, ProviderId::Codex, probe(ProviderId::Codex)),
        capped(PROBE_CAP, ProviderId::Gemini, probe(ProviderId::Gemini)),
        capped(PROBE_CAP, ProviderId::Grok, probe(ProviderId::Grok)),
    );
    vec![claude, deepseek, codex, gemini, grok]
}

/// Run one probe under `cap`. A probe that hangs must not delay startup, and "could
/// not tell" is safe: Unknown is not Ready, so nothing is routed there.
pub(super) async fn capped(cap: Duration, provider: ProviderId, probe: impl Future<Output = Availability>) -> Availability {
    match tokio::time::timeout(cap, probe).await {
        Ok(a) => a,
        Err(_) => avail(provider, State::Unknown, format!("probe timed out after {}s", cap.as_secs_f32())),
    }
}
