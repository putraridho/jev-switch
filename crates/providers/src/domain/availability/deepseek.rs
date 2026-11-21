//! The DeepSeek probe: the `claude` CLI plus a key.

use super::{avail, Availability, State};
use jev_backend::ProviderId;

/// DeepSeek is the `claude` CLI pointed at another API, so it needs both the binary
/// and a key.
pub fn decide_deepseek(claude_installed: bool, key_present: bool) -> Availability {
    let id = ProviderId::DeepSeek;
    if !claude_installed {
        return avail(id, State::NotInstalled, "needs the `claude` CLI, not found on PATH");
    }
    if !key_present {
        return avail(id, State::NoKey, "set DEEPSEEK_API_KEY (environment or ~/.jev-router.env)");
    }
    Availability { plan: Some("pay-per-token".into()), ..avail(id, State::Ready, "DEEPSEEK_API_KEY set") }
}
