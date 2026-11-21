//! Probes of the machine for the doctor: the Jev ping, `git`, the state directory, the key file.
//! Every probe is bounded and a failure becomes a fact, never a panic.

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::domain::diagnose::EnvFileFacts;
use jev_judgment::application::ports::JevTransport;
use jev_judgment::application::wire::{choice_question, decode_choice};
use jev_judgment::infrastructure::typesafe_http::TypeSafe;

/// The Jev ping and the `git` check each get this long, so `doctor` never hangs.
const IO_CAP: Duration = Duration::from_secs(3);

/// `(key present, reachable)`. Reachable is not asked without a key.
pub async fn jev_status() -> (bool, Option<bool>) {
    let Ok(client) = TypeSafe::from_env() else { return (false, None) };
    let question = choice_question("Pick option a.", json!({ "a": "the only real option", "b": "never pick this" }));
    let ping = client.ask_many(json!("ping"), json!({ "q": question }));
    let ok = matches!(tokio::time::timeout(IO_CAP, ping).await, Ok(Ok(answers)) if decode_choice(&answers, "q").is_ok());
    (true, Some(ok))
}

/// `Some(true/false)` from `git rev-parse`; `None` when git cannot be run at all.
pub async fn in_git_repo(dir: &Path) -> Option<bool> {
    let run = tokio::process::Command::new("git")
        .args(jev_shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status();
    match tokio::time::timeout(IO_CAP, run).await {
        Ok(Ok(status)) => Some(status.success()),
        _ => None,
    }
}

/// Create the state directory if needed and write then remove a scratch file.
pub fn state_dir_writable() -> bool {
    let Some(dir) = jev_shared::infrastructure::jsonl_log::state_dir() else { return false };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".doctor-probe-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn env_file_facts() -> Option<EnvFileFacts> {
    let path = jev_shared::infrastructure::env_file::env_file_path(std::env::var("JEV_ROUTER_ENV").ok(), std::env::var("HOME").ok())?;
    let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Some(EnvFileFacts { path, mode_bits: mode_bits(&meta), has_keys: file_has_keys(&text) })
}

#[cfg(unix)]
fn mode_bits(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_bits(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// True when some `KEY=value` line has a non-empty value (comments and blanks do not count).
fn file_has_keys(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim().trim_start_matches("export ").trim();
        !line.starts_with('#')
            && line.split_once('=').is_some_and(|(k, v)| !k.trim().is_empty() && !v.trim().trim_matches(|c| c == '"' || c == '\'').is_empty())
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_has_keys_ignores_comments_blanks_and_empty_values() {
        assert!(file_has_keys("JEV_API_KEY=abc\n"));
        assert!(file_has_keys("# note\nexport X='y'\n"));
        assert!(!file_has_keys(""));
        assert!(!file_has_keys("# A=1\n\nB=\nC=\"\"\nno equals\n"));
    }
}
