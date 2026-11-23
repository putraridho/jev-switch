//! Building the backend of a provider: which CLI, with which environment. The only place that
//! knows every concrete backend; the choices themselves are pure functions in
//! [`jev_backend::infrastructure::select`].

use std::path::PathBuf;

use jev_backend::infrastructure::claude::claude_cli::ClaudeBackend;
use jev_backend::infrastructure::codex::codex_cli::CodexBackend;
use jev_backend::infrastructure::gemini::gemini_cli::{Flavor, GeminiBackend};
use jev_backend::infrastructure::grok::grok_cli::GrokBackend;
use jev_backend::infrastructure::select::{choose_gemini_flavor, deepseek_env, gemini_env};
use jev_backend::{Backend, ProviderId};
use jev_shared::infrastructure::env_file as envfile;

/// A fresh backend for one turn. Codex and Gemini keep per-turn parse state inside
/// the instance, so sharing one across turns would mix their output: build a new one
/// every time (construction is cheap).
pub fn backend_for(provider: ProviderId) -> Box<dyn Backend> {
    build_backend(provider, false)
}

/// [`backend_for`] for the `jev-code` session: the Claude Code CLIs (Claude, DeepSeek)
/// ask the host about tool approvals and know where they run. The other CLIs are the same
/// as in [`backend_for`]: they have no host approval protocol, Codex's sandbox decides.
pub fn session_backend_for(provider: ProviderId) -> Box<dyn Backend> {
    build_backend(provider, true)
}

/// [`backend_for`] for a delegated worker, which nobody watches: Grok runs over ACP so its
/// shell commands and edits reach the worker's approver (headless Grok denies every shell
/// command on its own). The other CLIs are the same as in [`backend_for`]: Antigravity is
/// already ACP, Claude Code gets `--allowedTools`, Codex's sandbox decides.
pub fn worker_backend_for(provider: ProviderId) -> Box<dyn Backend> {
    match provider {
        ProviderId::Grok => Box::new(grok_backend().over_acp()),
        _ => backend_for(provider),
    }
}

/// Grok with the key from the shell or `~/.jev-router.env`; when the user is also signed in,
/// under a Grok home without the login so the key is the one used (see `key_home`).
fn grok_backend() -> GrokBackend {
    use jev_backend::infrastructure::grok::grok_cli::{auth_env, key_home};
    GrokBackend::grok().with_env(key_home::with_key_home(auth_env(&envfile::lookup), &envfile::lookup))
}

/// The backends of a `jev-code` run: [`session_backend_for`] when someone can answer an
/// approval question (`can_ask`: a session on a terminal), else [`backend_for`]. A run nobody
/// can answer (`-p`, piped input) must not use the host prompt protocol: every question would
/// be denied, and the model would be told "jev-code asks the user" when nobody will. Plain
/// `claude -p` decides with the allowed tools and the permission mode instead.
pub fn run_backends(can_ask: bool) -> fn(ProviderId) -> Box<dyn Backend> {
    if can_ask {
        session_backend_for
    } else {
        backend_for
    }
}

fn build_backend(provider: ProviderId, session: bool) -> Box<dyn Backend> {
    let claude_code = |backend: ClaudeBackend| -> Box<dyn Backend> {
        Box::new(if session { backend.in_session() } else { backend })
    };
    match provider {
        ProviderId::Claude => claude_code(ClaudeBackend::claude()),
        ProviderId::Codex => Box::new(CodexBackend::codex()),
        ProviderId::Grok => Box::new(grok_backend()),
        ProviderId::Gemini => Box::new(match detect_gemini_flavor() {
            Flavor::GeminiCli => GeminiBackend::gemini_cli().with_env(gemini_env(&envfile::lookup)),
            Flavor::Antigravity => GeminiBackend::antigravity().with_env(gemini_env(&envfile::lookup)),
        }),
        // No key means the caller never routes here; an empty key just fails the turn.
        ProviderId::DeepSeek => {
            let key = envfile::lookup("DEEPSEEK_API_KEY").unwrap_or_default();
            claude_code(ClaudeBackend::with_profile(ProviderId::DeepSeek, deepseek_env(&key)))
        }
    }
}

/// The CLI behind `gemini` right now, for display: `gemini` or `agy`.
pub fn gemini_via() -> &'static str {
    match detect_gemini_flavor() {
        Flavor::GeminiCli => "gemini",
        Flavor::Antigravity => "agy",
    }
}

fn detect_gemini_flavor() -> Flavor {
    let settings = std::env::var_os("HOME")
        .and_then(|h| std::fs::read_to_string(PathBuf::from(h).join(".gemini/settings.json")).ok());
    // The same rule the availability probe uses, so "ready" and "which CLI" agree.
    let configured = jev_backend::infrastructure::gemini::gemini_cli::auth::is_configured(settings.as_deref(), |n| envfile::lookup(n).is_some());
    let installed = crate::infrastructure::probe_host::installed_for_real;
    choose_gemini_flavor(installed(&GeminiBackend::antigravity().bin), installed(&GeminiBackend::gemini_cli().bin), configured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, DeepSeek};

    #[test]
    fn a_run_nobody_can_answer_never_uses_the_host_prompt_protocol() {
        let req = jev_backend::TurnRequest { dir: "/tmp".into(), model: "m".into(), prompt: "hi".into(), images: vec![], session_id: None, mode: Default::default(), trust_project_settings: false };
        let args = |b: &dyn Backend| -> Vec<String> { b.command(&req).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        for provider in [Claude, DeepSeek] {
            // `-p` or piped input: plain `claude -p`, allowed tools and permission mode decide.
            let plain = run_backends(false)(provider);
            let a = args(&*plain);
            assert!(!plain.interactive(), "{provider}");
            assert!(!a.iter().any(|x| x == "--permission-prompt-tool" || x == jev_backend::infrastructure::claude::claude_cli::SESSION_NOTE), "{provider}: {a:?}");
            assert!(a.iter().any(|x| x == "--permission-mode"), "{provider}: {a:?}");
            // A session on a terminal asks the person.
            let asking = run_backends(true)(provider);
            let a = args(&*asking);
            assert!(asking.interactive(), "{provider}");
            assert!(a.iter().any(|x| x == "--permission-prompt-tool") && a.iter().any(|x| x == jev_backend::infrastructure::claude::claude_cli::SESSION_NOTE), "{provider}: {a:?}");
        }
    }

    #[test]
    fn only_a_worker_runs_grok_over_acp() {
        let req = jev_backend::TurnRequest { dir: "/tmp".into(), model: "grok-4.7".into(), prompt: "hi".into(), images: vec![], session_id: None, mode: Default::default(), trust_project_settings: false };
        let args = |b: &dyn Backend| -> Vec<String> { b.command(&req).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let worker = worker_backend_for(ProviderId::Grok);
        assert!(worker.interactive());
        assert_eq!(args(&*worker), ["agent", "stdio"]);
        for plain in [backend_for(ProviderId::Grok), session_backend_for(ProviderId::Grok)] {
            assert!(!plain.interactive());
            assert!(args(&*plain).iter().any(|a| a.starts_with("--single=")), "{:?}", args(&*plain));
        }
    }

    #[test]
    fn agy_wins_whenever_installed_and_gemini_cli_needs_both_binary_and_auth() {
        // agy installed: always agy, whatever the old Gemini CLI has.
        for gemini in [true, false] {
            for configured in [true, false] {
                assert_eq!(choose_gemini_flavor(true, gemini, configured), Flavor::Antigravity, "gemini {gemini}, configured {configured}");
            }
        }
        // No agy: the Gemini CLI only with its binary and auth.
        assert_eq!(choose_gemini_flavor(false, true, true), Flavor::GeminiCli);
        assert_eq!(choose_gemini_flavor(false, true, false), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, false, true), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, false, false), Flavor::Antigravity);
    }

    #[test]
    fn deepseek_env_points_at_deepseek_with_the_key_twice() {
        let env = deepseek_env("sk-test");
        assert!(env.contains(&("ANTHROPIC_BASE_URL".to_string(), "https://api.deepseek.com/anthropic".to_string())));
        assert!(env.contains(&("ANTHROPIC_AUTH_TOKEN".to_string(), "sk-test".to_string())));
        assert!(env.contains(&("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())));
        assert_eq!(env.len(), 3);
    }

    #[test]
    fn gemini_env_picks_up_keys_from_either_source() {
        let lookup = |n: &str| (n == "GEMINI_API_KEY").then(|| "from-file".to_string());
        assert_eq!(gemini_env(&lookup), vec![("GEMINI_API_KEY".to_string(), "from-file".to_string())]);
        assert!(gemini_env(&|_| None).is_empty());
    }
}
