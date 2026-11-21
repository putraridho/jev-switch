//! One check per provider, and the exact steps to get a provider from its state to ready.

use jev_providers::domain::availability::{Availability, State};
use jev_providers::domain::login_hints::{install_hint, login_command};
use jev_shared::ProviderId;

use super::{check, steps, Check, Severity};

const UNVERIFIED_NOTE: &str = "unverified: this check was written from documentation, not tried against the real CLI";

pub(super) fn no_provider_check(ready: &[ProviderId]) -> Option<Check> {
    ready.is_empty().then(|| {
        check(
            "no-provider",
            "No provider is ready",
            Severity::Problem,
            "Every turn needs at least one provider that is installed and logged in.",
            steps(&["Set up one provider using the steps for it below"]),
        )
    })
}

pub(super) fn provider_check(a: &Availability, env_path: &str) -> Check {
    let name = a.provider.as_str();
    let (severity, title, mut fix) = match a.state {
        State::Ready => (Severity::Ok, format!("{name} is ready"), Vec::new()),
        State::NotInstalled => (Severity::Info, format!("{name} is not installed"), fix_steps(a.provider, a.state, env_path)),
        State::NotLoggedIn => (Severity::Warn, format!("{name} is installed but not logged in"), fix_steps(a.provider, a.state, env_path)),
        State::NoKey => (Severity::Warn, format!("{name} has no API key"), fix_steps(a.provider, a.state, env_path)),
        State::Unknown => (Severity::Warn, format!("{name}: could not tell if it is usable"), fix_steps(a.provider, a.state, env_path)),
    };
    fix.dedup();
    check(provider_id(a.provider), title, severity, provider_detail(a), fix)
}

/// One stable id per provider, for renderers that want to find a check.
pub(super) fn provider_id(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Claude => "provider-claude",
        ProviderId::DeepSeek => "provider-deepseek",
        ProviderId::Codex => "provider-codex",
        ProviderId::Gemini => "provider-gemini",
        ProviderId::Grok => "provider-grok",
    }
}

/// State, plan, models and the probe's own words, plus an unverified label.
fn provider_detail(a: &Availability) -> String {
    let mut parts: Vec<String> = Vec::new();
    if a.state == State::Ready {
        if let Some(plan) = &a.plan {
            parts.push(format!("plan: {plan}"));
        }
        match &a.models {
            Some(models) if !models.is_empty() => parts.push(format!("models: {}", models.join(", "))),
            _ => parts.push("models: all the router knows for it".to_string()),
        }
    }
    if !a.detail.is_empty() {
        parts.push(a.detail.clone());
    }
    if a.provider == ProviderId::DeepSeek && a.state != State::NotInstalled {
        parts.push("billed per token; adding a key creates no subscription".to_string());
    }
    if !a.verified {
        parts.push(format!("({UNVERIFIED_NOTE})"));
    }
    parts.join("; ")
}

/// The exact steps to get `provider` from `state` to ready. Installing comes first,
/// then logging in. Steps that are commands are the command alone; anything not
/// tried against the real CLI says "unverified".
pub(super) fn fix_steps(provider: ProviderId, state: State, env_path: &str) -> Vec<String> {
    match state {
        State::Ready => Vec::new(),
        State::NotInstalled => [install_steps(provider), login_steps(provider, env_path)].concat(),
        State::NotLoggedIn | State::NoKey => login_steps(provider, env_path),
        State::Unknown => unknown_steps(provider),
    }
}

fn install_steps(provider: ProviderId) -> Vec<String> {
    vec![install_hint(provider).to_string()]
}

pub fn login_steps(provider: ProviderId, env_path: &str) -> Vec<String> {
    let cmd = login_command(provider).unwrap_or_default();
    match provider {
        ProviderId::Claude => vec![cmd.to_string()],
        ProviderId::Codex => vec![cmd.to_string(), format!("{cmd} --device-auth (if this machine is headless)")],
        ProviderId::Gemini => vec![
            format!("Run `{cmd}` once and sign in"),
            format!("or set GEMINI_API_KEY=<key> in {env_path} (unverified: the probe reads an exported GEMINI_API_KEY today; if Gemini stays not ready, `export GEMINI_API_KEY=<key>` in your shell)"),
            "Gemini CLI's own login is retired by Google; `gemini` is used only where `agy` is not installed".to_string(),
        ],
        ProviderId::Grok => vec![
            format!("{cmd} (first run installs it, which can take minutes)"),
            "Sign in when it asks (unverified; needs SuperGrok or X Premium+, the $8 plan likely lacks the CLI)".to_string(),
            format!("or set XAI_API_KEY=<key> in {env_path} (billed per token by xAI)"),
        ],
        ProviderId::DeepSeek => vec![format!("Add DEEPSEEK_API_KEY=<key> to {env_path} (billed per token)")],
    }
}

fn unknown_steps(provider: ProviderId) -> Vec<String> {
    let status = match provider {
        ProviderId::Claude => Some("claude auth status"),
        ProviderId::Codex => Some("codex login status"),
        _ => None,
    };
    status.into_iter().map(String::from).chain(["/providers refresh".to_string()]).collect()
}
