//! Gemini and Antigravity error wording.

use crate::ErrorKind;

/// How long until a quota resets, from Gemini's "Your quota will reset after
/// 3h20m5s." (the shared classifier only knows the "resets ..." wording).
pub(super) fn reset_after(message: &str) -> Option<String> {
    const KEY: &str = "reset after ";
    // ASCII lowercasing keeps byte offsets identical to the original.
    let at = message.to_ascii_lowercase().find(KEY)? + KEY.len();
    let word = message[at..].split_whitespace().next()?.trim_end_matches(['.', ',']);
    (!word.is_empty()).then(|| word.to_string())
}

/// What only these CLIs say: their capacity and auth wording, and a blank answer.
pub(super) fn classify_text(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase();
    let has = |phrases: &[&str]| phrases.iter().any(|p| lower.contains(p));
    if has(&["resource_exhausted", "exhausted your capacity", "too many requests"]) {
        Some(ErrorKind::Quota { resets_at: None })
    } else if has(&[
        "authentication failed",
        "authentication required",
        "authentication error",
        "not authenticated",
        "auth method",
        "please sign in",
        "sign in to",
        "api key not valid",
        "api_key_invalid",
    ]) {
        Some(ErrorKind::Auth)
    } else if has(&["empty response", "empty answer"]) {
        Some(ErrorKind::Empty)
    } else if has(&["no longer supported", "client is no longer", "migrate to the antigravity"]) {
        Some(ErrorKind::Unsupported)
    } else if has(&["not trusted", "trust_workspace", "trust this folder", "trust this directory"]) {
        Some(ErrorKind::UnusableHere)
    } else {
        None
    }
}
