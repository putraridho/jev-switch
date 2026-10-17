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
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// `env_var` if set, else `file` inside [`state_dir`].
pub fn path_from_env(env_var: &str, file: &str) -> Option<String> {
    std::env::var(env_var)
        .ok()
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
    rotate_if_big(path);
    let mut line = record.to_string();
    line.push('\n');
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}
