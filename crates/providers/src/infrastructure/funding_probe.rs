//! The adapters that find out how each provider's CLI is funded (see
//! [`crate::domain::funding`]): they read the CLI's own configuration and credential
//! metadata through [`AccountFacts`], keep only the kind, and let the pure rules decide.
//! No secret is returned, logged or put in a label.

use std::path::Path;

use jev_backend::infrastructure::gemini::gemini_cli::auth::selected_auth_type;
use jev_backend::infrastructure::grok::grok_cli::API_KEY_ENV as XAI_KEY;
use jev_backend::ProviderId;
use jev_shared::ALL_PROVIDERS;
use serde_json::Value;

use crate::application::ports::AccountFacts;
use crate::domain::funding::codex::{codex_access, CodexAuth};
use crate::domain::funding::{Access, Funding, Fundings};

/// Every provider's funding.
pub fn detect(facts: &dyn AccountFacts) -> Fundings {
    ALL_PROVIDERS.into_iter().fold(Fundings::default(), |all, provider| all.with(provider, detect_one(facts, provider)))
}

/// One provider's funding.
pub fn detect_one(facts: &dyn AccountFacts, provider: ProviderId) -> Funding {
    let access = match provider {
        ProviderId::Claude => claude(facts),
        ProviderId::Codex => codex(facts),
        ProviderId::Gemini => gemini(facts),
        ProviderId::Grok => grok(facts),
        // Always a key billed per token, and the key is what makes it ready.
        ProviderId::DeepSeek => Access { api_key: true, ..Access::default() },
    };
    Funding::from_access(access)
}

/// Claude Code uses `ANTHROPIC_API_KEY` or an auth token over the saved login, and a base URL
/// (or Bedrock/Vertex) sends it elsewhere. Only the process environment reaches the CLI.
/// Its login is not a file the router can see, so without those it is taken as the login.
fn claude(facts: &dyn AccountFacts) -> Access {
    let any = |names: &[&str]| names.iter().any(|n| facts.process_env_set(n));
    Access {
        custom_endpoint: any(&["ANTHROPIC_BASE_URL", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX"]),
        api_key: any(&["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]),
        login: true,
    }
}

fn codex(facts: &dyn AccountFacts) -> Access {
    let home = facts.codex_home();
    let config = home.as_ref().and_then(|h| facts.read_text(&h.join("config.toml")));
    let auth = home.as_ref().and_then(|h| facts.read_text(&h.join("auth.json"))).map_or(CodexAuth::Missing, |t| codex_auth(&t));
    codex_access(config.as_deref(), auth)
}

/// The sign-in kind from `auth.json`: its `auth_mode`, else which credential it holds.
pub(crate) fn codex_auth(auth_json: &str) -> CodexAuth {
    let Ok(auth) = serde_json::from_str::<Value>(auth_json) else { return CodexAuth::Missing };
    let non_empty = |v: &Value| v.as_str().is_some_and(|s| !s.trim().is_empty());
    match auth["auth_mode"].as_str().map(|m| m.to_ascii_lowercase().replace('_', "")).as_deref() {
        Some("chatgpt") => CodexAuth::ChatGpt,
        Some("apikey") => CodexAuth::ApiKey,
        _ if auth["tokens"].is_object() => CodexAuth::ChatGpt,
        _ if non_empty(&auth["OPENAI_API_KEY"]) => CodexAuth::ApiKey,
        _ => CodexAuth::Missing,
    }
}

/// An API key or Vertex setting is billed per token; the Code Assist flag and the saved
/// Google or Antigravity logins are the account's own.
fn gemini(facts: &dyn AccountFacts) -> Access {
    let home = facts.home_dir();
    let under = |rel: &str| home.as_ref().map(|h| h.join(rel));
    let selected = under(".gemini/settings.json").and_then(|p| facts.read_text(&p)).and_then(|t| selected_auth_type(&t));
    let key_type = selected.as_deref().is_some_and(|t| t.contains("api-key") || t.contains("vertex"));
    let login_type = selected.is_some() && !key_type;
    let exists = |rel: &str| under(rel).is_some_and(|p| facts.exists(&p));
    Access {
        custom_endpoint: false,
        api_key: key_type || ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_USE_VERTEXAI"].iter().any(|n| facts.setting_set(n)),
        login: login_type || facts.setting_set("GOOGLE_GENAI_USE_GCA") || exists(".gemini/oauth_creds.json") || exists(".gemini/antigravity-cli"),
    }
}

/// The Grok CLI prefers a saved login to the key, but jev-code gives it the key alone when
/// one is set (`key_home`), so a key wins here as it does at run time.
fn grok(facts: &dyn AccountFacts) -> Access {
    let login = facts.home_dir().is_some_and(|h| facts.exists(&h.join(".grok").join("auth.json")));
    Access { custom_endpoint: false, api_key: facts.setting_set(XAI_KEY), login }
}

/// The real machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemAccountFacts;

impl AccountFacts for SystemAccountFacts {
    fn setting_set(&self, name: &str) -> bool {
        jev_shared::infrastructure::env_file::lookup(name).is_some_and(|v| !v.trim().is_empty())
    }

    fn process_env_set(&self, name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| !v.is_empty())
    }

    fn home_dir(&self) -> Option<std::path::PathBuf> {
        super::probe_host::home()
    }

    fn codex_home(&self) -> Option<std::path::PathBuf> {
        jev_backend::infrastructure::codex::codex_plan::codex_home()
    }

    fn read_text(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

#[cfg(test)]
mod tests;
