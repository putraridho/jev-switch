//! The Gemini probe: two CLIs (`gemini`, `agy`) and several ways of being signed in.

use super::{avail, Availability, State};
use jev_backend::ProviderId;

/// Everything the Gemini decision needs. Two CLIs serve it (`gemini` and `agy`), and
/// each has several ways of being signed in, so the facts are listed rather than
/// computed in the decision.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeminiFacts {
    pub gemini_installed: bool,
    pub agy_installed: bool,
    /// An API-key or Vertex/Code Assist variable in the environment (`gemini::AUTH_ENV_VARS`).
    pub api_key_env: bool,
    /// `~/.gemini/settings.json` names an auth type.
    pub settings_auth: bool,
    /// Cached OAuth login, `~/.gemini/oauth_creds.json`.
    pub oauth_creds: bool,
    /// The agy login directory `~/.gemini/antigravity-cli/` has files in it.
    pub agy_creds: bool,
}

/// Which sign-in was found, for the detail line.
pub fn gemini_auth_source(f: &GeminiFacts) -> Option<&'static str> {
    [
        (f.api_key_env, "API key in environment"),
        (f.settings_auth, "auth type set in ~/.gemini/settings.json"),
        (f.oauth_creds, "cached OAuth login"),
        (f.agy_creds, "agy login"),
    ]
    .into_iter()
    .find_map(|(present, what)| present.then_some(what))
}

pub fn decide_gemini(f: GeminiFacts) -> Availability {
    let id = ProviderId::Gemini;
    if !f.gemini_installed && !f.agy_installed {
        return avail(id, State::NotInstalled, "neither `gemini` nor `agy` found on PATH");
    }
    match gemini_auth_source(&f) {
        Some(source) => avail(id, State::Ready, source),
        None => avail(id, State::NotLoggedIn, "no API key, settings auth type or login found (`gemini` exits 41 without auth)"),
    }
}
