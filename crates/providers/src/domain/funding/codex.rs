//! What Codex's `config.toml` says about the backend its CLI will use. Pure text rules; the
//! file itself is read by the adapter.

use super::Access;

/// How Codex is signed in, from `auth.json` metadata. Never the credential itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexAuth {
    ChatGpt,
    ApiKey,
    Missing,
}

/// The built-in provider id; Codex sends it to OpenAI with the ChatGPT login or an API key.
const BUILT_IN: &str = "openai";

/// Whether the effective configuration routes Codex away from the built-in OpenAI provider:
/// a `model_provider` (top level, or the one the selected `profile` sets) other than
/// `openai`, or an `openai_base_url` override. Such a backend bills by its own rules.
pub fn config_selects_custom_backend(config_toml: &str) -> bool {
    let mut table = String::new();
    let (mut top_provider, mut profile, mut base_url) = (None, None, false);
    let mut profile_providers: Vec<(String, String)> = Vec::new();
    for raw in config_toml.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(header) = line.strip_prefix('[') {
            table = header.trim_end_matches(']').trim().trim_matches('[').trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim().trim_matches('"'), value.trim().trim_matches(['"', '\'']).to_string());
        match (table.as_str(), key) {
            ("", "model_provider") => top_provider = Some(value),
            ("", "profile") => profile = Some(value),
            ("", "openai_base_url") => base_url = !value.is_empty(),
            (t, "model_provider") => {
                if let Some(name) = t.strip_prefix("profiles.") {
                    profile_providers.push((name.trim_matches('"').to_string(), value));
                }
            }
            _ => {}
        }
    }
    let chosen = profile
        .and_then(|p| profile_providers.into_iter().find(|(name, _)| *name == p).map(|(_, provider)| provider))
        .or(top_provider);
    base_url || chosen.is_some_and(|p| p != BUILT_IN)
}

/// The facts of Codex: `config` is `config.toml`'s text when it could be read.
pub fn codex_access(config: Option<&str>, auth: CodexAuth) -> Access {
    Access {
        custom_endpoint: config.is_some_and(config_selects_custom_backend),
        api_key: auth == CodexAuth::ApiKey,
        login: auth == CodexAuth::ChatGpt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::funding::Funding;

    #[test]
    fn the_built_in_provider_is_not_custom() {
        assert!(!config_selects_custom_backend(""));
        assert!(!config_selects_custom_backend("model = \"gpt\"\nmodel_provider = \"openai\"\n"));
    }

    #[test]
    fn a_custom_model_provider_is_custom() {
        let toml = "model_provider = \"jitera\"\n\n[model_providers.jitera]\nname = \"j\"\nbase_url = \"https://controller.example.com/v1\"\n";
        assert!(config_selects_custom_backend(toml));
        assert!(config_selects_custom_backend("openai_base_url = \"https://proxy.example\"\n"));
    }

    #[test]
    fn a_provider_table_alone_does_not_select_it() {
        assert!(!config_selects_custom_backend("[model_providers.jitera]\nbase_url = \"https://x\"\n"));
    }

    #[test]
    fn the_selected_profile_overrides_the_top_level() {
        let toml = "model_provider = \"openai\"\nprofile = \"work\"\n[profiles.work]\nmodel_provider = \"azure\"\n[profiles.other]\nmodel_provider = \"openai\"\n";
        assert!(config_selects_custom_backend(toml));
        let back = "model_provider = \"azure\"\nprofile = \"home\"\n[profiles.home]\nmodel_provider = \"openai\"\n";
        assert!(!config_selects_custom_backend(back));
    }

    #[test]
    fn auth_mode_and_config_decide_the_funding() {
        let funding = |config, auth| Funding::from_access(codex_access(config, auth));
        assert_eq!(funding(None, CodexAuth::ChatGpt), Funding::Login);
        assert_eq!(funding(None, CodexAuth::ApiKey), Funding::ApiKey);
        assert_eq!(funding(None, CodexAuth::Missing), Funding::Unknown);
        assert_eq!(funding(Some("model_provider = \"jitera\""), CodexAuth::ChatGpt), Funding::Custom);
    }
}
