//! The Grok probe, decided from the filesystem alone (see the module docs).

use super::host::home;
use super::mise::installed_for_real;
use super::{avail, Availability, State};
use crate::backend::infrastructure::grok::grok_cli as grok;
use crate::backend::ProviderId;
use crate::shared::infrastructure::env_file as envfile;

/// Grok is decided from the filesystem alone (see the module docs).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct GrokFacts {
    /// A real binary exists, not just the lazy mise command-wrapper.
    pub(super) real_binary: bool,
    /// `~/.grok` exists: the CLI has been signed in or used.
    pub(super) home_dir: bool,
    /// `XAI_API_KEY` is set in the environment or `~/.jev-router.env`.
    pub(super) api_key: bool,
}

pub(super) fn decide_grok(f: GrokFacts) -> Availability {
    let id = ProviderId::Grok;
    match (f.real_binary, f.api_key, f.home_dir) {
        (false, _, _) => avail(id, State::NotInstalled, "no installed `grok` (only a lazy mise wrapper, if anything)"),
        (true, true, _) => avail(id, State::Ready, "installed, XAI_API_KEY set"),
        (true, false, true) => avail(id, State::Ready, "installed, ~/.grok present"),
        (true, false, false) => avail(id, State::NotLoggedIn, "installed, but no ~/.grok or XAI_API_KEY: sign in or set the key"),
    }
}

pub(super) async fn probe_grok() -> Availability {
    decide_grok(GrokFacts {
        real_binary: installed_for_real("grok"),
        home_dir: home().is_some_and(|h| grok::session_dir_exists(&h)),
        // The same rule the provider factory uses to pass the key on, so "ready" means it reaches the CLI.
        api_key: !grok::auth_env(&envfile::lookup).is_empty(),
    })
}
