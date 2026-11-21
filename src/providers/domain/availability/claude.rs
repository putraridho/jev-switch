//! The Claude probe: `claude auth status`.

use serde_json::Value;

use super::{avail, Availability, State};
use crate::backend::ProviderId;

/// What `claude auth status` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeLogin {
    pub(crate) logged_in: bool,
    pub(crate) plan: Option<String>,
}

/// `claude auth status` prints JSON by default (`loggedIn`, `planTier`,
/// `subscriptionType`); the text form is read too so a changed default does not
/// break the probe. `None` when neither says anything clear.
pub(crate) fn parse_claude_login(text: &str) -> Option<ClaudeLogin> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        let logged_in = v.get("loggedIn")?.as_bool()?;
        let plan = ["planTier", "subscriptionType"]
            .iter()
            .find_map(|k| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(str::to_string);
        return Some(ClaudeLogin { logged_in, plan });
    }
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || lower.contains("not authenticated") {
        Some(ClaudeLogin { logged_in: false, plan: None })
    } else if lower.contains("logged in") {
        Some(ClaudeLogin { logged_in: true, plan: None })
    } else {
        None
    }
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
