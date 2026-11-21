//! Machine facts used by provider probes.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::domain::availability::host::find_on_path;
use crate::domain::availability::mise::{is_mise_wrapper, mise_data_dir, really_installed};

fn is_executable_file(path: &Path) -> bool {
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

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn read_home_file(rel: &str) -> Option<String> {
    std::fs::read_to_string(home()?.join(rel)).ok()
}

pub fn home_path_exists(rel: &str) -> bool {
    home().is_some_and(|h| h.join(rel).exists())
}

pub fn installed_for_real(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable_file(Path::new(bin)) && !is_mise_wrapper(Path::new(bin));
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let candidates = find_on_path(bin, &dirs, &is_executable_file);
    let data = mise_data_dir(std::env::var("MISE_DATA_DIR").ok(), std::env::var("XDG_DATA_HOME").ok(), home());
    really_installed(&candidates, data.is_some_and(|d| mise_installs_binary(&d, bin)))
}

pub fn mise_installs_binary(data_dir: &Path, bin: &str) -> bool {
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
    };
    let versions = subdirs(&data_dir.join("installs")).iter().flat_map(|tool| subdirs(tool)).collect::<Vec<_>>();
    crate::domain::availability::mise::mise_installs_binary(&versions, bin, &is_executable_file)
}

pub struct Captured {
    pub ok: bool,
    pub text: String,
}

pub fn probe_command(program: &str, args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).env("MISE_OFFLINE", "1").stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

pub async fn run_capture(program: &str, args: &[&str]) -> Option<Captured> {
    let out = probe_command(program, args).output().await.ok()?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some(Captured { ok: out.status.success(), text })
}
