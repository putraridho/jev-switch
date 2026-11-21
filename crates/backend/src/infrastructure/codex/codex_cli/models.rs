//! The models a Codex login can pick.

use serde_json::Value;

/// Models this login can pick, from the text of `~/.codex/models_cache.json`: the
/// slugs whose `visibility` is `list` (the rest are hidden or deprecated). Unreadable
/// or unexpected JSON gives an empty list, which callers treat as "unknown".
pub fn allowed_models_from_cache(json: &str) -> Vec<String> {
    let Ok(cache) = serde_json::from_str::<Value>(json) else { return vec![] };
    let models = cache["models"].as_array().into_iter().flatten();
    models
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| m["slug"].as_str().map(String::from))
        .collect()
}
