//! The Codex probe: `codex login status`, the plan from `auth.json`, the model cache.

use super::{avail, Availability, State};
use jev_backend::ProviderId;

/// `codex login status`: `Some(true/false)` when its output is clear. A non-zero
/// exit means not logged in; "Not logged in" is checked first because it contains
/// "logged in".
pub fn parse_codex_login(ok: bool, text: &str) -> Option<bool> {
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || !ok {
        Some(false)
    } else if lower.contains("logged in") {
        Some(true)
    } else {
        None
    }
}

pub fn decode_base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' | '+' => 62,
            '_' | '/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// `login_ok` is `None` when `codex login status` could not be run or read.
pub fn decide_codex(installed: bool, login_ok: Option<bool>, plan: Option<&str>, models: Option<Vec<String>>) -> Availability {
    let id = ProviderId::Codex;
    if !installed {
        return avail(id, State::NotInstalled, "`codex` not found on PATH");
    }
    match login_ok {
        None => avail(id, State::Unknown, "could not read `codex login status`"),
        Some(false) => avail(id, State::NotLoggedIn, "run `codex login`"),
        Some(true) => Availability {
            plan: plan.map(str::to_string),
            models: models.filter(|m| !m.is_empty()),
            ..avail(id, State::Ready, "logged in with ChatGPT")
        },
    }
}
