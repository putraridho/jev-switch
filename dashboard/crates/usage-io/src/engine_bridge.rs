//! The engine port (DESIGN 5.1): the ONLY file that names the engine. Five small facts come
//! from the engine's facade; no engine type appears in any signature here, and every other
//! Flashboard module calls these functions and never the engine.

use crate::envfile;
use jev_engine as engine;
use jev_usage_core::provider::ProviderKey;
use std::path::{Path, PathBuf};

/// The process variable and env-file key that override the usage log's path.
const USAGE_LOG_VAR: &str = "JEV_USAGE_LOG";
/// The usage log's file name inside the state directory.
const USAGE_LOG_FILE: &str = "usage.jsonl";
/// The process variable that overrides the env file's path.
const ENV_FILE_VAR: &str = "JEV_ROUTER_ENV";

/// The gateway's state directory (`$XDG_STATE_HOME/jev-router`, else
/// `~/.local/state/jev-router`). The engine creates it, owner-only, when it is missing.
pub fn state_dir() -> Option<PathBuf> {
    engine::state_dir()
}

/// `~/.jev-router.env`, or the path `JEV_ROUTER_ENV` names: the facade's `env_file_path`
/// over the real environment. The file need not exist.
pub fn env_file() -> Option<PathBuf> {
    engine::env_file_path(std::env::var(ENV_FILE_VAR).ok(), std::env::var("HOME").ok())
}

/// Pure: the value of one key in env-file text, by the engine's rule (`export`, one layer
/// of quotes, an empty value skipped).
pub fn parse_env(text: &str, name: &str) -> Option<String> {
    engine::parse_env(text, name)
}

/// Pure: the engine's rule for `codex login status`. `None` when the output is not clear.
pub fn codex_logged_in(exit_ok: bool, text: &str) -> Option<bool> {
    engine::parse_codex_login(exit_ok, text)
}

/// The engine's login command for a dashboard provider (DESIGN 4.1): `codex login`,
/// `claude auth login`; `None` where Flashboard offers none. Only the two providers whose
/// CLI the app itself runs are mapped to the engine's provider id.
pub fn login_command(p: ProviderKey) -> Option<&'static str> {
    let id = match p {
        ProviderKey::Claude => engine::ProviderId::Claude,
        ProviderKey::Codex => engine::ProviderId::Codex,
        ProviderKey::DeepSeek
        | ProviderKey::Gemini
        | ProviderKey::Antigravity
        | ProviderKey::Grok => return None,
    };
    engine::login_command(id)
}

/// Pure and tested: the process variable (an empty one counts as unset), else the env-file
/// key, else `<state_dir>/usage.jsonl`. The same order as the engine's own log path rule,
/// without its unzeroized file reader.
pub fn usage_log_path_from(
    env: Option<String>,
    env_file: Option<&Path>,
    state_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(value) = env.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(value));
    }
    if let Some(value) = env_file.and_then(|path| envfile::read_value(path, USAGE_LOG_VAR)) {
        return Some(PathBuf::from(value.as_str()));
    }
    state_dir.map(|dir| dir.join(USAGE_LOG_FILE))
}

/// [`usage_log_path_from`] over the real environment. The state directory is only asked
/// for (and so only created) when neither override names a path.
pub fn usage_log_path() -> Option<PathBuf> {
    let file = env_file();
    usage_log_path_from(std::env::var(USAGE_LOG_VAR).ok(), file.as_deref(), None)
        .or_else(|| usage_log_path_from(None, None, state_dir()))
}

/// Flashboard's own directory: `<state_dir>/dashboard`, or `dashboard-dev` in a debug
/// build, so a development run never shares files with the installed app (DESIGN 8.8).
pub fn dashboard_dir() -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) {
        "dashboard-dev"
    } else {
        "dashboard"
    };
    state_dir().map(|dir| dir.join(name))
}
