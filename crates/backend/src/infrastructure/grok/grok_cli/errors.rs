//! Grok-specific error wording on top of the shared classification rules.

use crate::{classify_common, has_any, ErrorKind};

/// The plan gate shows up as a refusal that names Grok's plan, not as a limit. Phrases
/// only: "subscription", "premium" and "required" appear together in ordinary billing
/// code ("error: subscription id is required"). Wording that is only a gate in a Grok
/// context ("subscription is required", "not included in your") needs Grok or X Premium
/// named as well.
pub(super) fn is_entitlement_text(lower: &str) -> bool {
    const ALWAYS: [&str; 4] = ["requires a supergrok", "supergrok subscription", "upgrade to supergrok", "x premium+"];
    const NEEDS_GROK: [&str; 6] = [
        "not included in your",
        "subscription is required",
        "subscription required",
        "no active subscription",
        "upgrade to",
        "only available to",
    ];
    has_any(lower, &ALWAYS) || (has_any(lower, &["grok", "x premium"]) && has_any(lower, &NEEDS_GROK))
}

/// Grok's own login wording; the shared rules cover "not logged in", "invalid api key"
/// and "authentication failed|required". "xai_api_key" counts only next to a word that
/// says it is missing or wrong, never on its own (docs and config mention it freely).
pub(super) fn is_auth_text(lower: &str) -> bool {
    let key_broken = has_any(lower, &["missing", "not set", "invalid", "not found", "is required"]);
    has_any(lower, &["grok login", "not authenticated", "api key is missing", "api key is invalid", "api key not set"])
        || (lower.contains("xai_api_key") && key_broken)
}

pub(super) fn classify(message: &str) -> ErrorKind {
    if let Some(kind) = classify_common(message) {
        return kind;
    }
    let lower = message.to_lowercase();
    if is_entitlement_text(&lower) {
        ErrorKind::NotEntitled
    } else if is_auth_text(&lower) {
        ErrorKind::Auth
    } else {
        ErrorKind::Other
    }
}
