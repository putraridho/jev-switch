//! Which CLI and environment run a provider: the DeepSeek wiring of the `claude` CLI and the
//! choice between Gemini CLI and Antigravity. Must not read the machine itself: callers pass facts.

use crate::backend::gemini::Flavor;

const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/anthropic";

/// Environment that points the `claude` CLI at DeepSeek's Anthropic-compatible API.
///
/// The key goes in both `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY` because some
/// Claude Code versions read one and some the other.
///
/// UNVERIFIED: that `claude -p` with these variables bypasses the user's Claude
/// login and really talks to DeepSeek. The availability probe only checks that the
/// key exists, not that this wiring works.
pub fn deepseek_env(key: &str) -> Vec<(String, String)> {
    vec![
        ("ANTHROPIC_BASE_URL".to_string(), DEEPSEEK_BASE_URL.to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), key.to_string()),
        ("ANTHROPIC_API_KEY".to_string(), key.to_string()),
    ]
}

/// Gemini CLI when it is installed and has auth; otherwise Antigravity. Gemini CLI
/// without auth exits immediately, so installed alone is not enough to prefer it.
pub fn choose_gemini_flavor(gemini_on_path: bool, gemini_configured: bool) -> Flavor {
    if gemini_on_path && gemini_configured {
        Flavor::GeminiCli
    } else {
        Flavor::Antigravity
    }
}

/// Gathers the two facts [`choose_gemini_flavor`] needs from the real machine.
/// The Gemini auth variables that are set in the environment OR in `~/.jev-router.env`,
/// so a key kept only in that file still reaches the CLI (and the probe agrees).
pub fn gemini_env(lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    crate::backend::gemini::AUTH_ENV_VARS
        .iter()
        .filter_map(|n| lookup(n).map(|v| (n.to_string(), v)))
        .collect()
}
