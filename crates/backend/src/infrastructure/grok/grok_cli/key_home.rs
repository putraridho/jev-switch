//! Making Grok use `XAI_API_KEY` when the user is also signed in.
//!
//! The Grok CLI prefers a saved login (`<grok home>/auth.json`) over `XAI_API_KEY`: its own
//! rule is "cached_token overrides xai.api_key", and a harness cannot change that. The
//! `GROK_CONFIG` overlay refuses auth settings, and `auth.preferred_method` exists only in the
//! user's `config.toml`. A user who set the key for jev-code would otherwise be billed to
//! the free or SuperGrok plan of the login and hit its limit.
//!
//! So a turn with a key runs under `GROK_HOME=<state>/grok-key-home`. That folder holds one
//! symlink per entry of the real Grok home, except `auth.json*`. Grok then finds no login and
//! uses the key, while sessions, config, skills and memory stay the user's own. Writes go
//! through the links, so a session started here can be resumed from plain `grok`.

use std::io;
use std::path::{Path, PathBuf};

use super::API_KEY_ENV;

/// The variable that moves Grok's home folder.
pub const HOME_ENV: &str = "GROK_HOME";
/// The mirror's folder name, under jev-code's state folder.
const MIRROR_DIR: &str = "grok-key-home";

/// Whether `name` is part of the saved login (`auth.json`, its locks and temp files).
fn is_login_file(name: &str) -> bool {
    name.starts_with("auth.json")
}

/// The Grok home the user's `grok` reads: `GROK_HOME` when set, else `<home>/.grok`.
pub fn real_home(lookup: &dyn Fn(&str) -> Option<String>, home: Option<&Path>) -> Option<PathBuf> {
    match lookup(HOME_ENV).filter(|v| !v.trim().is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.map(|h| h.join(".grok")),
    }
}

/// Whether a turn has to run under the mirror: a key is passed and a saved login would win
/// over it.
pub fn needs_mirror(env: &[(String, String)], real: &Path) -> bool {
    env.iter().any(|(k, _)| k == API_KEY_ENV) && real.join("auth.json").exists()
}

/// Bring `mirror` in line with `real`: a link for every entry of `real` except the login,
/// with no stale links and no login of its own. Run before every turn, which is cheap
/// (one listing of each folder), so an entry Grok added to the real home since the last
/// turn shows up.
pub fn sync_mirror(real: &Path, mirror: &Path) -> io::Result<()> {
    std::fs::create_dir_all(mirror)?;
    for entry in std::fs::read_dir(mirror)? {
        let entry = entry?;
        let name = entry.file_name();
        let path = entry.path();
        let is_link = entry.file_type()?.is_symlink();
        let wanted = !is_login_file(&name.to_string_lossy()) && real.join(&name).symlink_metadata().is_ok();
        if is_link && !wanted {
            std::fs::remove_file(&path)?;
        } else if !is_link && is_login_file(&name.to_string_lossy()) {
            // A login written here (a `/login` inside a routed turn) would beat the key again.
            remove_any(&path)?;
        }
    }
    for entry in std::fs::read_dir(real)? {
        let name = entry?.file_name();
        if is_login_file(&name.to_string_lossy()) {
            continue;
        }
        let link = mirror.join(&name);
        if link.symlink_metadata().is_err() {
            symlink(&real.join(&name), &link)?;
        }
    }
    Ok(())
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// `env` plus `GROK_HOME=<mirror>` when the key has to beat a saved login. Any failure
/// (no state folder, a folder that cannot be written) leaves `env` as it was: Grok then
/// runs as before, on the login.
pub fn with_key_home(mut env: Vec<(String, String)>, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(real) = real_home(lookup, home.as_deref()) else { return env };
    if !needs_mirror(&env, &real) {
        return env;
    }
    let Some(mirror) = jev_shared::infrastructure::jsonl_log::state_dir().map(|d| d.join(MIRROR_DIR)) else { return env };
    if sync_mirror(&real, &mirror).is_ok() {
        env.push((HOME_ENV.to_string(), mirror.to_string_lossy().into_owned()));
    }
    env
}
