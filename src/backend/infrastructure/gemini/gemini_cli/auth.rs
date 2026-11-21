//! Is the Gemini CLI set up: environment variables or a `settings.json` auth type.

use serde_json::Value;

/// Environment variables that make Gemini CLI authenticate without `settings.json`
/// (read from its `getAuthTypeFromEnv`). Only [`is_configured`] reads this list.
pub const AUTH_ENV_VARS: [&str; 4] =
    ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_USE_VERTEXAI", "GOOGLE_GENAI_USE_GCA"];

/// THE answer to "is the Gemini CLI set up": an API-key or Vertex/Code Assist
/// environment variable (see [`AUTH_ENV_VARS`]; `env_set` says whether a variable
/// is set) or a `settings.json` that names an auth type. The availability probe and
/// the provider factory both ask this, so they cannot disagree. Pure so it can be
/// tested without touching real files or the environment. Antigravity keeps no such
/// setting here, so this says nothing about `agy`.
pub fn is_configured(gemini_settings_json: Option<&str>, env_set: impl Fn(&str) -> bool) -> bool {
    auth_looks_configured(gemini_settings_json, AUTH_ENV_VARS.iter().any(|n| env_set(n)))
}

/// The settings-or-flag half of [`is_configured`]; private, callers go through that.
///
/// Gemini CLI accepts comments in `settings.json`, so whole-line `//` comments are
/// dropped before parsing. Unreadable JSON counts as not configured.
pub(super) fn auth_looks_configured(gemini_settings_json: Option<&str>, env_has_key: bool) -> bool {
    env_has_key || gemini_settings_json.is_some_and(names_auth_type)
}

fn names_auth_type(json: &str) -> bool {
    let uncommented: String =
        json.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let Ok(settings) = serde_json::from_str::<Value>(&uncommented) else { return false };
    // `security.auth.selectedType` is current; `selectedAuthType` is the older top-level key.
    let named = settings["security"]["auth"]["selectedType"].as_str().or(settings["selectedAuthType"].as_str());
    named.is_some_and(|t| !t.trim().is_empty())
}
