//! The Grok probe, decided from the filesystem alone (see the module docs).

use super::{avail, Availability, State};
use jev_backend::ProviderId;

/// Grok is decided from the filesystem alone (see the module docs).
#[derive(Debug, Clone, Copy, Default)]
pub struct GrokFacts {
    /// A real binary exists, not just the lazy mise command-wrapper.
    pub real_binary: bool,
    /// `~/.grok` exists: the CLI has been signed in or used.
    pub home_dir: bool,
    /// `XAI_API_KEY` is set in the environment or `~/.jev-router.env`.
    pub api_key: bool,
}

pub fn decide_grok(f: GrokFacts) -> Availability {
    let id = ProviderId::Grok;
    match (f.real_binary, f.api_key, f.home_dir) {
        (false, _, _) => avail(id, State::NotInstalled, "no installed `grok` (only a lazy mise wrapper, if anything)"),
        (true, true, _) => avail(id, State::Ready, "installed, XAI_API_KEY set"),
        (true, false, true) => avail(id, State::Ready, "installed, ~/.grok present"),
        (true, false, false) => avail(id, State::NotLoggedIn, "installed, but no ~/.grok or XAI_API_KEY: sign in or set the key"),
    }
}
