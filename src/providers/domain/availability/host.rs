//! The thin IO helpers the probes share: PATH search, home files, a status CLI.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Every executable called `name` in `dirs`, in PATH order. `is_exec` is injected so
/// the search is testable without a filesystem.
pub(super) fn find_on_path(name: &str, dirs: &[PathBuf], is_exec: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    dirs.iter().map(|d| d.join(name)).filter(|p| is_exec(p)).collect()
}

pub(super) fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

pub(super) fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

pub(super) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(super) fn read_home_file(rel: &str) -> Option<String> {
    std::fs::read_to_string(home()?.join(rel)).ok()
}

pub(super) fn home_path_exists(rel: &str) -> bool {
    home().is_some_and(|h| h.join(rel).exists())
}

// ---------------------------------------------------------------- run a status CLI

/// What a short status command printed. `text` is stdout then stderr, because CLIs
/// disagree on which stream carries the answer.
pub(super) struct Captured {
    pub(super) ok: bool,
    pub(super) text: String,
}

/// Run `program args` with no stdin, and wait for it. `None` when it cannot start.
/// The child is killed if the probe is cancelled by the time cap.
/// A status probe's command. Probes run in mise's offline mode: on Omarchy the agent CLIs
/// are mise launchers that update themselves before they run (at most every 4 hours), and a
/// probe that triggers that update blows its 5 s cap (the `? codex` at startup) and can make
/// mise install other tools. Offline, the installed version answers in well under a second
/// (measured 0.4 s for `codex login status`, 0.6 s for `claude auth status`) and nothing is
/// fetched. Real turns are NOT run offline: they behave exactly as when the user runs the tool.
pub(super) fn probe_command(program: &str, args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).env("MISE_OFFLINE", "1").stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

pub(super) async fn run_capture(program: &str, args: &[&str]) -> Option<Captured> {
    let out = probe_command(program, args).output().await.ok()?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some(Captured { ok: out.status.success(), text })
}
