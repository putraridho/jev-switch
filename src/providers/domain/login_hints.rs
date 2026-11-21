//! What to tell the user about installing a provider's CLI and logging in to it: the
//! one source of install hints, login commands and the "log in again" wording.

use crate::backend::ProviderId;

/// Where keys live when `$JEV_ROUTER_ENV` does not say otherwise; shown in fixes.
pub const DEFAULT_ENV_PATH: &str = "~/.jev-router.env";

/// How to install the CLI behind `provider`: the one source of install hints.
pub fn install_hint(provider: ProviderId) -> &'static str {
    match provider {
        // DeepSeek is the claude CLI pointed at another API.
        ProviderId::Claude | ProviderId::DeepSeek => "Install Claude Code: https://code.claude.com/docs/en/quickstart",
        ProviderId::Codex => "npm i -g @openai/codex (or see the vendor's install page)",
        ProviderId::Gemini => "npm i -g @google/gemini-cli",
        ProviderId::Grok => {
            "Install the Grok CLI: see the vendor's install page (if `grok` is a lazy mise wrapper, running `grok` once installs it)"
        }
    }
}

/// The command that logs in to `provider`: the one source of login commands, used by
/// `doctor`, the first-run hint and every error message. `None` for DeepSeek, which has
/// no login: it takes an API key in the env file. Only `claude auth login` and `codex
/// login` were tried against the real CLIs ([`login_verified`]).
pub fn login_command(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some("claude auth login"),
        ProviderId::Codex => Some("codex login"),
        // Running the CLI starts its own sign-in flow.
        ProviderId::Gemini => Some("gemini"),
        ProviderId::Grok => Some("grok"),
        ProviderId::DeepSeek => None,
    }
}

/// Whether [`login_command`] for `provider` was run against the real CLI.
pub fn login_verified(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex)
}

/// [`login_command`] as one sentence for a message: `run `codex login``.
pub fn login_instruction(provider: ProviderId) -> String {
    match (provider, login_command(provider)) {
        (ProviderId::Gemini, Some(cmd)) => format!("run `{cmd}` and choose a login"),
        (ProviderId::Grok, Some(cmd)) => format!("run `{cmd}` and sign in"),
        (_, Some(cmd)) => format!("run `{cmd}`"),
        (_, None) => format!("add `DEEPSEEK_API_KEY` to {DEFAULT_ENV_PATH}"),
    }
}

/// What the user can do about a provider blocked as unsupported (`BlockReason::Unsupported`) for `provider`.
pub(crate) fn unsupported_hint(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Gemini => "is retired by Google: install Antigravity (agy) and log in there, then restart jev-code",
        _ => "is no longer accepted by its service: update or replace the CLI, then restart jev-code",
    }
}

/// What the user can do about a provider blocked as unusable here (`BlockReason::UnusableHere`) for `provider`.
pub(crate) fn unusable_here_hint(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "needs a git repository here (git init, or work in a repo)",
        ProviderId::Gemini => "needs a trusted workspace here (run gemini in this directory once to trust it)",
        _ => "refuses to run in this directory",
    }
}

/// The tool a user logs in to again for `provider`.
pub fn login_target(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Gemini => "gemini (or agy)",
        ProviderId::Grok => "grok",
        ProviderId::DeepSeek => "DeepSeek: check DEEPSEEK_API_KEY",
    }
}
