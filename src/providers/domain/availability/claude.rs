//! The Claude probe: `claude auth status`.

use super::{avail, Availability, State};
use crate::backend::ProviderId;

/// What `claude auth status` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeLogin {
    pub(crate) logged_in: bool,
    pub(crate) plan: Option<String>,
}

pub(crate) fn decide_claude(installed: bool, login: Option<ClaudeLogin>) -> Availability {
    let id = ProviderId::Claude;
    if !installed {
        return avail(id, State::NotInstalled, "`claude` not found on PATH");
    }
    match login {
        None => avail(id, State::Unknown, "could not read `claude auth status`"),
        Some(l) if !l.logged_in => avail(id, State::NotLoggedIn, "run `claude auth login`"),
        Some(l) => Availability { plan: l.plan, ..avail(id, State::Ready, "logged in") },
    }
}
