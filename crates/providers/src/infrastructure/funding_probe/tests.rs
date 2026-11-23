use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

/// A machine in a map: files by path, settings, and the process's own environment.
#[derive(Debug, Default)]
struct Fake {
    files: HashMap<PathBuf, String>,
    settings: HashSet<&'static str>,
    process: HashSet<&'static str>,
}

const HOME: &str = "/home/u";

impl Fake {
    fn file(mut self, rel: &str, text: &str) -> Self {
        self.files.insert(Path::new(HOME).join(rel), text.to_string());
        self
    }
    fn setting(mut self, name: &'static str) -> Self {
        self.settings.insert(name);
        self
    }
    fn process_env(mut self, name: &'static str) -> Self {
        self.process.insert(name);
        self
    }
}

impl AccountFacts for Fake {
    fn setting_set(&self, name: &str) -> bool {
        self.settings.contains(name) || self.process.contains(name)
    }
    fn process_env_set(&self, name: &str) -> bool {
        self.process.contains(name)
    }
    fn home_dir(&self) -> Option<PathBuf> {
        Some(PathBuf::from(HOME))
    }
    fn codex_home(&self) -> Option<PathBuf> {
        Some(Path::new(HOME).join(".codex"))
    }
    fn read_text(&self, path: &Path) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
}

const CHATGPT: &str = r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x"}}"#;

#[test]
fn codex_on_a_chatgpt_login_is_a_login() {
    let f = Fake::default().file(".codex/auth.json", CHATGPT).file(".codex/config.toml", "model = \"gpt-6.1-terra\"\n");
    assert_eq!(detect_one(&f, Codex), Funding::Login);
}

/// The reproduction from the inspection report: the login probe says "logged in with ChatGPT"
/// while the configuration selects another model provider.
#[test]
fn codex_with_a_custom_model_provider_is_custom_whatever_the_login_says() {
    let toml = "model_provider = \"jitera\"\n[model_providers.jitera]\nbase_url = \"https://controller.example.com/v1\"\n";
    let f = Fake::default().file(".codex/auth.json", CHATGPT).file(".codex/config.toml", toml);
    assert_eq!(detect_one(&f, Codex), Funding::Custom);
}

#[test]
fn codex_auth_kinds_come_from_metadata_only() {
    assert_eq!(codex_auth(r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-secret"}"#), CodexAuth::ApiKey);
    assert_eq!(codex_auth(r#"{"auth_mode":"ApiKey"}"#), CodexAuth::ApiKey);
    assert_eq!(codex_auth(r#"{"OPENAI_API_KEY":"sk-secret"}"#), CodexAuth::ApiKey);
    assert_eq!(codex_auth(r#"{"OPENAI_API_KEY":null,"tokens":{}}"#), CodexAuth::ChatGpt);
    assert_eq!(codex_auth("nope"), CodexAuth::Missing);
    assert_eq!(codex_auth("{}"), CodexAuth::Missing);
}

#[test]
fn codex_without_auth_is_unknown_not_a_login() {
    assert_eq!(detect_one(&Fake::default(), Codex), Funding::Unknown);
}

#[test]
fn grok_key_beats_a_saved_login() {
    let login = Fake::default().file(".grok/auth.json", "{}");
    assert_eq!(detect_one(&login, Grok), Funding::Login);
    assert_eq!(detect_one(&login.setting("XAI_API_KEY"), Grok), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().setting("XAI_API_KEY"), Grok), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().file(".grok/sessions", ""), Grok), Funding::Unknown, "a folder alone is not a login");
}

#[test]
fn gemini_key_vertex_and_logins() {
    assert_eq!(detect_one(&Fake::default().setting("GEMINI_API_KEY"), Gemini), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().setting("GOOGLE_GENAI_USE_VERTEXAI"), Gemini), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().file(".gemini/oauth_creds.json", "{}"), Gemini), Funding::Login);
    assert_eq!(detect_one(&Fake::default().file(".gemini/antigravity-cli", ""), Gemini), Funding::Login);
    let oauth = r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#;
    assert_eq!(detect_one(&Fake::default().file(".gemini/settings.json", oauth), Gemini), Funding::Login);
    let key = r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#;
    assert_eq!(detect_one(&Fake::default().file(".gemini/settings.json", key).file(".gemini/oauth_creds.json", "{}"), Gemini), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default(), Gemini), Funding::Unknown);
}

#[test]
fn claude_key_or_endpoint_beats_the_login() {
    assert_eq!(detect_one(&Fake::default(), Claude), Funding::Login);
    assert_eq!(detect_one(&Fake::default().process_env("ANTHROPIC_API_KEY"), Claude), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().process_env("ANTHROPIC_AUTH_TOKEN"), Claude), Funding::ApiKey);
    assert_eq!(detect_one(&Fake::default().process_env("ANTHROPIC_BASE_URL").process_env("ANTHROPIC_API_KEY"), Claude), Funding::Custom);
    assert_eq!(detect_one(&Fake::default().setting("ANTHROPIC_API_KEY"), Claude), Funding::Login, "the key file does not reach the CLI");
}

#[test]
fn deepseek_is_always_per_token() {
    assert_eq!(detect_one(&Fake::default(), DeepSeek), Funding::ApiKey);
}

#[test]
fn detect_covers_every_provider() {
    let all = detect(&Fake::default());
    assert!(ALL_PROVIDERS.iter().all(|p| all.get(*p).is_some()));
}
