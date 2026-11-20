//! Append-only JSON-lines logs under `~/.local/state/jev-router` (or
//! `$XDG_STATE_HOME/jev-router`), for later analysis with jq, DuckDB, etc.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Directory holding the logs, created on first use.
pub fn state_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    let dir = base.join("jev-router");
    make_private_dir(&dir).ok()?;
    Some(dir)
}

/// Create `dir` (and missing parents) readable by the owner only. The folder holds prompts,
/// sessions and the audit log, so other local users may not even list it; one an earlier
/// version made with the default umask is tightened.
fn make_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700).create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if meta.is_dir() && meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    builder.create(dir)
}

/// `env_var` if set, else `file` inside [`state_dir`].
pub fn path_from_env(env_var: &str, file: &str) -> Option<String> {
    // The environment first, then the env file, like every other setting.
    super::envfile::lookup(env_var)
        .or_else(|| Some(state_dir()?.join(file).to_string_lossy().into_owned()))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Size at which a log is moved aside to `<path>.1` (replacing the previous one);
/// `JEV_LOG_MAX_MB` overrides, 0 disables.
fn max_bytes() -> u64 {
    let mb = std::env::var("JEV_LOG_MAX_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(50);
    mb * 1024 * 1024
}

fn rotate_if_big(path: &str) {
    let max = max_bytes();
    if max > 0 && std::fs::metadata(path).is_ok_and(|m| m.len() >= max) {
        let _ = std::fs::rename(path, format!("{path}.1"));
    }
}

/// Append one record as a line. Each line goes out in a single write so records
/// from concurrent requests don't interleave. Never fails the caller.
pub fn append(path: &str, record: &Value) {
    append_line(path, &record.to_string());
}

/// [`append`] for a line that is already text (a typed record serialised by its owner).
pub fn append_line(path: &str, text: &str) {
    rotate_if_big(path);
    let mut line = text.to_string();
    line.push('\n');
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}
