//! The machine half of the provider probes: gathers the facts (PATH, status CLIs, home files,
//! settings) and hands them to the pure `decide_*` rules in `providers/domain/availability`.
//! Also [`SystemProbes`], the adapter of the registry's [`ProviderHost`] port.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use jev_backend::infrastructure::gemini::gemini_cli::auth as gemini;
use jev_backend::infrastructure::grok::grok_cli as grok;
use jev_backend::{PlanUsage, ProviderId};
use crate::application::ports::{BoxFuture, ProviderHost};
use crate::domain::availability::claude::decide_claude;
use crate::domain::availability::codex::{decide_codex, parse_codex_login};
use super::probe_wire::{codex_plan_from_auth, parse_claude_login};
use crate::domain::availability::deepseek::decide_deepseek;
use crate::domain::availability::gemini::{decide_gemini, GeminiFacts};
use crate::domain::availability::grok::{decide_grok, GrokFacts};
use super::probe_host::{home, home_path_exists, read_home_file, run_capture, installed_for_real};
use crate::domain::availability::Availability;
use jev_shared::application::host::Host;
use jev_shared::infrastructure::env_file as envfile;
use jev_shared::infrastructure::system_host::SystemHost;

/// One provider's probe, with no time cap of its own: the caller says how long it may take.
pub async fn probe_provider(provider: ProviderId) -> Availability {
    match provider {
        ProviderId::Claude => probe_claude().await,
        ProviderId::DeepSeek => probe_deepseek().await,
        ProviderId::Codex => probe_codex().await,
        ProviderId::Gemini => probe_gemini().await,
        ProviderId::Grok => probe_grok().await,
    }
}

async fn probe_claude() -> Availability {
    let installed = installed_for_real("claude");
    let login = match installed {
        true => run_capture("claude", &["auth", "status"]).await.and_then(|c| parse_claude_login(&c.text)),
        false => None,
    };
    decide_claude(installed, login)
}

async fn probe_codex() -> Availability {
    let installed = installed_for_real("codex");
    if !installed {
        return decide_codex(false, None, None, None);
    }
    let login_ok = run_capture("codex", &["login", "status"]).await.and_then(|c| parse_codex_login(c.ok, &c.text));
    let plan = read_home_file(".codex/auth.json").and_then(|t| codex_plan_from_auth(&t));
    let models = read_home_file(".codex/models_cache.json").map(|t| jev_backend::infrastructure::codex::codex_cli::models::allowed_models_from_cache(&t));
    decide_codex(installed, login_ok, plan.as_deref(), models)
}

async fn probe_deepseek() -> Availability {
    decide_deepseek(installed_for_real("claude"), envfile::lookup("DEEPSEEK_API_KEY").is_some())
}

/// The two Gemini facts that come from the settings file and the environment,
/// both decided by [`gemini::is_configured`], the rule the provider factory uses
/// too. Returns `(api_key_env, settings_auth)`.
fn gemini_login_facts(settings: Option<&str>, env_set: impl Fn(&str) -> bool) -> (bool, bool) {
    (gemini::is_configured(None, env_set), gemini::is_configured(settings, |_| false))
}

fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

async fn probe_gemini() -> Availability {
    let settings = read_home_file(".gemini/settings.json");
    let (api_key_env, settings_auth) = gemini_login_facts(settings.as_deref(), |n| envfile::lookup(n).is_some());
    decide_gemini(GeminiFacts {
        gemini_installed: installed_for_real("gemini"),
        agy_installed: installed_for_real("agy"),
        api_key_env,
        settings_auth,
        oauth_creds: home_path_exists(".gemini/oauth_creds.json"),
        agy_creds: home().is_some_and(|h| dir_has_entries(&h.join(".gemini/antigravity-cli"))),
    })
}

async fn probe_grok() -> Availability {
    decide_grok(GrokFacts {
        real_binary: installed_for_real("grok"),
        home_dir: home().is_some_and(|h| grok::session_dir_exists(&h)),
        // The same rule the provider factory uses to pass the key on, so "ready" means it reaches the CLI.
        api_key: !grok::auth_env(&envfile::lookup).is_empty(),
    })
}

/// The real machine: the probes above, the probe cache and plan files, Codex's session files
/// and DeepSeek's balance endpoint.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProbes;

impl ProviderHost for SystemProbes {
    fn load_cooldowns(&self, path: &Path) -> crate::domain::reset_time::cooldowns::Cooldowns {
        super::cooldown_file::load(path)
    }

    fn save_cooldowns(&self, path: &Path, cooldowns: &crate::domain::reset_time::cooldowns::Cooldowns, now_ms: u64) -> std::io::Result<crate::domain::reset_time::cooldowns::Cooldowns> {
        super::cooldown_file::save(path, cooldowns, now_ms)
    }

    fn reported_windows(&self) -> HashMap<(ProviderId, String), u64> {
        jev_backend::infrastructure::codex::codex_plan::codex_home().map(|home| super::codex_models::windows_in(&home.join("models_cache.json"))).unwrap_or_default()
    }

    fn load_breakers(&self, path: &Path) -> crate::domain::breaker::Breakers {
        super::health_file::load(path)
    }

    fn save_breakers(&self, path: &Path, breakers: &crate::domain::breaker::Breakers) -> std::io::Result<crate::domain::breaker::Breakers> {
        super::health_file::save(path, breakers)
    }

    fn cooldown_stamp(&self, path: &Path) -> Option<(std::time::SystemTime, u64)> {
        super::cooldown_file::stamp(path)
    }

    fn resolve_reset_time(&self, text: &str, zone: Option<&str>) -> Option<u64> {
        super::clock_facts::system_date(text, zone)
    }

    fn reset_zone_exists(&self, zone: &str) -> bool {
        super::clock_facts::system_zone_exists(zone)
    }
    fn probe(&self, provider: ProviderId) -> BoxFuture<Availability> {
        Box::pin(probe_provider(provider))
    }

    fn load_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64) -> Option<Vec<Availability>> {
        super::probe_cache::load(path, now_ms, fingerprint)
    }

    fn save_probe_cache(&self, path: &Path, now_ms: u64, fingerprint: u64, avail: &[Availability]) {
        super::probe_cache::save(path, now_ms, fingerprint, avail);
    }

    fn plan_path(&self) -> Option<PathBuf> {
        super::plan_store::default_path()
    }

    fn load_plans(&self, path: &Path) -> HashMap<ProviderId, PlanUsage> {
        super::plan_store::load(path)
    }

    fn save_plans(&self, path: &Path, plans: &HashMap<ProviderId, PlanUsage>) {
        super::plan_store::save(path, plans);
    }

    fn plan_is_current(&self, plan: &PlanUsage, now_s: u64) -> bool {
        super::plan_store::is_current(plan, now_s)
    }

    fn latest_codex_plan(&self) -> Option<PlanUsage> {
        jev_backend::infrastructure::codex::codex_plan::latest_plan().map(|(plan, _)| plan)
    }

    fn fetch_credit(&self, key: String) -> BoxFuture<Option<String>> {
        Box::pin(async move { super::deepseek_balance::fetch_deepseek_balance(&key).await.as_ref().map(super::deepseek_balance::format_balance) })
    }

    fn env(&self, name: &str) -> Option<String> {
        SystemHost.env(name)
    }

    fn now_ms(&self) -> u64 {
        SystemHost.now_ms()
    }

    fn state_dir(&self) -> Option<PathBuf> {
        SystemHost.state_dir()
    }

    fn fundings(&self) -> crate::domain::funding::Fundings {
        super::funding_probe::detect(&super::funding_probe::SystemAccountFacts)
    }
}

impl crate::application::ports::ReadinessHost for SystemProbes {
    fn readiness_key(&self, provider: ProviderId) -> crate::domain::readiness::ReadinessKey {
        super::readiness_facts::key_of(provider)
    }

    fn project_key(&self, provider: ProviderId, dir: &Path) -> String {
        super::readiness_facts::project_key(provider, dir)
    }

    fn handshake(&self, provider: ProviderId, dir: PathBuf) -> BoxFuture<crate::domain::handshake::Handshake> {
        Box::pin(super::readiness_facts::handshake(provider, dir))
    }

    fn load_readiness(&self, path: &Path) -> crate::domain::readiness::Readiness {
        super::readiness_file::load(path)
    }

    fn save_readiness(&self, path: &Path, readiness: &crate::domain::readiness::Readiness) -> std::io::Result<crate::domain::readiness::Readiness> {
        super::readiness_file::save(path, readiness)
    }
}

#[cfg(test)]
mod tests {
    use super::gemini_login_facts;

    #[test]
    fn gemini_login_facts_follow_the_factory_rule() {
        let commented = "{\n  // login\n  \"security\": {\"auth\": {\"selectedType\": \"oauth-personal\"}}\n}";
        let no_env = |_: &str| false;
        assert_eq!(gemini_login_facts(Some(commented), no_env), (false, true), "comments in settings.json are fine");
        assert_eq!(gemini_login_facts(None, |n| n == "GOOGLE_GENAI_USE_VERTEXAI"), (true, false), "Vertex env only");
        assert_eq!(gemini_login_facts(None, |n| n == "GOOGLE_GENAI_USE_GCA"), (true, false));
        assert_eq!(gemini_login_facts(Some(r#"{"theme":"dark"}"#), no_env), (false, false));
        assert_eq!(gemini_login_facts(Some("nope"), no_env), (false, false));
        assert_eq!(gemini_login_facts(None, no_env), (false, false));
    }
}
