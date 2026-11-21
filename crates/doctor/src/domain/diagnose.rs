//! `doctor`: what is set up, what is not, and the exact thing to do about each gap.
//!
//! The decision logic is pure: [`diagnose`] turns an [`Inputs`] snapshot into a
//! [`Report`] of structured [`Check`]s, and [`first_run_hint`] turns provider
//! availability into the short startup nudge. Neither does IO, so both are
//! table-tested. Only [`gather`] touches the machine (file metadata, a scratch write,
//! `git`, one tiny Jev request), and it never panics: a failed probe is a fact
//! (`Some(false)`), not an error. Colors and layout belong to whoever renders the report.

use std::cmp::Reverse;
use std::path::PathBuf;

use jev_providers::domain::availability::{Availability, State};
use jev_providers::domain::login_hints::DEFAULT_ENV_PATH;
use jev_shared::ProviderId;

mod jev_checks;
mod machine_checks;
pub mod provider_checks;
pub mod startup_hint;
mod state_checks;
#[cfg(test)]
mod tests;

use jev_checks::{jev_key_check, jev_reachable_check};
use machine_checks::{env_perms_check, git_check, state_dir_check};
use provider_checks::{no_provider_check, provider_check};
use state_checks::{blocked_check, cooldown_check, credits_check};

// ------------------------------------------------------------------ report types

/// Most severe is greatest, so sorting descending puts Problems first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: &'static str,
    pub title: String,
    pub severity: Severity,
    pub detail: String,
    /// Exact commands or steps, in order. Empty when there is nothing to do.
    pub fix: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Problems first, then Warns, then Infos, then Oks; stable within a severity.
    pub checks: Vec<Check>,
    pub ready_providers: Vec<ProviderId>,
    /// One plain sentence on what routing can do right now.
    pub routing_note: String,
}

/// What `~/.jev-router.env` (or `$JEV_ROUTER_ENV`) looks like on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvFileFacts {
    pub path: PathBuf,
    /// Permission bits (`mode & 0o777`); `None` where the platform has none.
    pub mode_bits: Option<u32>,
    pub has_keys: bool,
}

/// Every fact [`diagnose`] needs. `None` means "not checked": the check is left out
/// rather than guessed.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub avail: Vec<Availability>,
    pub jev_key_present: bool,
    pub jev_reachable: Option<bool>,
    pub in_git_repo: Option<bool>,
    pub state_dir_writable: Option<bool>,
    pub env_file: Option<EnvFileFacts>,
    /// Active cooldowns, one human line each ("claude cooling for 12m").
    pub cooldown_lines: Vec<String>,
    /// Providers skipped for this session, with why.
    pub blocked: Vec<(ProviderId, String)>,
    pub allow_credits: bool,
}

// ---------------------------------------------------------------------- diagnose

/// The whole report. Pure.
pub fn diagnose(inputs: &Inputs) -> Report {
    let env_path = env_path_text(inputs);
    let ready = ready_providers(inputs);

    let mut checks: Vec<Check> = Vec::new();
    checks.extend(no_provider_check(&ready));
    checks.push(jev_key_check(inputs.jev_key_present, &env_path));
    checks.extend(jev_reachable_check(inputs));
    checks.extend(inputs.avail.iter().map(|a| provider_check(a, &env_path)));
    checks.extend(git_check(inputs.in_git_repo));
    checks.extend(state_dir_check(inputs.state_dir_writable));
    checks.extend(env_perms_check(inputs.env_file.as_ref()));
    checks.extend(credits_check(inputs));
    checks.extend(cooldown_check(&inputs.cooldown_lines));
    checks.extend(blocked_check(&inputs.blocked));

    // Stable, so the construction order above breaks ties within a severity.
    checks.sort_by_key(|c| Reverse(c.severity));

    let routing_note = routing_note(&ready);
    Report { checks, ready_providers: ready, routing_note }
}

/// Ready providers that are not blocked for this session, in availability order.
fn ready_providers(inputs: &Inputs) -> Vec<ProviderId> {
    inputs
        .avail
        .iter()
        .filter(|a| a.state == State::Ready)
        .map(|a| a.provider)
        .filter(|p| !inputs.blocked.iter().any(|(b, _)| b == p))
        .collect()
}

fn routing_note(ready: &[ProviderId]) -> String {
    match ready {
        [] => "No provider is ready, so nothing can run yet. Set one up using the steps below.".to_string(),
        [only] => format!("Only {only} is ready, so every turn runs on {only}. Add a second provider to route by task type."),
        many => format!("Routing across {}.", many.iter().map(ProviderId::as_str).collect::<Vec<_>>().join(", ")),
    }
}

fn env_path_text(inputs: &Inputs) -> String {
    inputs.env_file.as_ref().map_or_else(|| DEFAULT_ENV_PATH.to_string(), |f| f.path.display().to_string())
}

fn check(id: &'static str, title: impl Into<String>, severity: Severity, detail: impl Into<String>, fix: Vec<String>) -> Check {
    Check { id, title: title.into(), severity, detail: detail.into(), fix }
}

fn steps(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
