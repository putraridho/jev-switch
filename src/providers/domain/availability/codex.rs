//! The Codex probe: `codex login status`, the plan from `auth.json`, the model cache.

use serde_json::Value;

use super::host::{read_home_file, run_capture};
use super::mise::installed_for_real;
use super::{avail, Availability, State};
use crate::backend::ProviderId;

/// `codex login status`: `Some(true/false)` when its output is clear. A non-zero
/// exit means not logged in; "Not logged in" is checked first because it contains
/// "logged in".
pub(super) fn parse_codex_login(ok: bool, text: &str) -> Option<bool> {
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || !ok {
        Some(false)
    } else if lower.contains("logged in") {
        Some(true)
    } else {
        None
    }
}

pub(super) fn decode_base64url(s: &str) -> Option<Vec<u8>> {
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

/// The ChatGPT plan ("free", "plus", ...) from `~/.codex/auth.json`: the
/// `chatgpt_plan_type` claim inside the `id_token` JWT. The token is only read, not
/// verified: the plan is shown to the user, never trusted for a decision.
pub(super) fn codex_plan_from_auth(auth_json: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_json).ok()?;
    let payload = auth.pointer("/tokens/id_token")?.as_str()?.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&decode_base64url(payload)?).ok()?;
    let plan = claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")?.as_str()?;
    (!plan.is_empty()).then(|| plan.to_string())
}

/// `login_ok` is `None` when `codex login status` could not be run or read.
pub(super) fn decide_codex(installed: bool, login_ok: Option<bool>, plan: Option<&str>, models: Option<Vec<String>>) -> Availability {
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

pub(super) async fn probe_codex() -> Availability {
    let installed = installed_for_real("codex");
    if !installed {
        return decide_codex(false, None, None, None);
    }
    let login_ok = run_capture("codex", &["login", "status"]).await.and_then(|c| parse_codex_login(c.ok, &c.text));
    let plan = read_home_file(".codex/auth.json").and_then(|t| codex_plan_from_auth(&t));
    let models = read_home_file(".codex/models_cache.json").map(|t| crate::backend::infrastructure::codex::codex_cli::models::allowed_models_from_cache(&t));
    decide_codex(installed, login_ok, plan.as_deref(), models)
}
