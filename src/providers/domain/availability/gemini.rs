//! The Gemini probe: two CLIs (`gemini`, `agy`) and several ways of being signed in.

use std::path::Path;

use super::host::{home, home_path_exists, read_home_file};
use super::mise::installed_for_real;
use super::{avail, Availability, State};
use crate::backend::infrastructure::gemini::gemini_cli::auth as gemini;
use crate::backend::ProviderId;
use crate::shared::infrastructure::env_file as envfile;

/// Everything the Gemini decision needs. Two CLIs serve it (`gemini` and `agy`), and
/// each has several ways of being signed in, so the facts are listed rather than
/// computed in the decision.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct GeminiFacts {
    pub(super) gemini_installed: bool,
    pub(super) agy_installed: bool,
    /// An API-key or Vertex/Code Assist variable in the environment (`gemini::AUTH_ENV_VARS`).
    pub(super) api_key_env: bool,
    /// `~/.gemini/settings.json` names an auth type.
    pub(super) settings_auth: bool,
    /// Cached OAuth login, `~/.gemini/oauth_creds.json`.
    pub(super) oauth_creds: bool,
    /// The agy login directory `~/.gemini/antigravity-cli/` has files in it.
    pub(super) agy_creds: bool,
}

/// The two Gemini facts that come from the settings file and the environment,
/// both decided by [`gemini::is_configured`], the rule the provider factory uses
/// too. Returns `(api_key_env, settings_auth)`.
pub(super) fn gemini_login_facts(settings: Option<&str>, env_set: impl Fn(&str) -> bool) -> (bool, bool) {
    (gemini::is_configured(None, env_set), gemini::is_configured(settings, |_| false))
}

/// Which sign-in was found, for the detail line.
pub(super) fn gemini_auth_source(f: &GeminiFacts) -> Option<&'static str> {
    [
        (f.api_key_env, "API key in environment"),
        (f.settings_auth, "auth type set in ~/.gemini/settings.json"),
        (f.oauth_creds, "cached OAuth login"),
        (f.agy_creds, "agy login"),
    ]
    .into_iter()
    .find_map(|(present, what)| present.then_some(what))
}

pub(super) fn decide_gemini(f: GeminiFacts) -> Availability {
    let id = ProviderId::Gemini;
    if !f.gemini_installed && !f.agy_installed {
        return avail(id, State::NotInstalled, "neither `gemini` nor `agy` found on PATH");
    }
    match gemini_auth_source(&f) {
        Some(source) => avail(id, State::Ready, source),
        None => avail(id, State::NotLoggedIn, "no API key, settings auth type or login found (`gemini` exits 41 without auth)"),
    }
}

pub(super) fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

pub(super) async fn probe_gemini() -> Availability {
    let settings = read_home_file(".gemini/settings.json");
    let (api_key_env, settings_auth) = gemini_login_facts(settings.as_deref(), |n| envfile::lookup(n).is_some());
    decide_gemini(GeminiFacts {
        gemini_installed: installed_for_real("gemini"),
        agy_installed: installed_for_real("agy"),
        api_key_env,
        settings_auth,
        oauth_creds: home_path_exists(".gemini/oauth_creds.json"),
        agy_creds: home().is_some_and(|h| dir_has_entries(&h.join(".gemini/antigravity-cli"))),
    })
}
