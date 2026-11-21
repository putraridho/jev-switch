//! `git status --short`, for the privacy check and the catch-up text.

use std::path::Path;

/// `git status --short` in `dir`; empty on any failure (not a repo, no git).
pub fn git_status_short(dir: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(jev_shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(["status", "--short"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}
