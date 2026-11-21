//! The doors every use case has to the machine it runs on: settings (environment and key
//! file), the clock, the state directory, private file writes and git's safety options.
//!
//! Owns the port ([`Host`]) and the one process-wide place its adapter is looked up. Must not
//! touch the environment, the clock or the disk itself: the real adapter is
//! [`crate::shared::infrastructure::system_host::SystemHost`], and a test (or a binary that
//! wants other answers) calls [`install`] with its own before the first use.
//!
//! This is the staged form of the plan's `Env`, `Clock` and `StateDir` ports (DDD.md P3-1):
//! use cases ask [`env`], [`now_ms`] and the other free functions here instead of naming an
//! infrastructure module, so they depend only on this file. Passing the port through each
//! constructor (P3-2) can then replace the lookup one component at a time.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What the machine can tell or do for a use case.
pub trait Host: Send + Sync {
    /// A setting: the environment first, then the key file; empty counts as unset.
    fn env(&self, name: &str) -> Option<String>;
    /// Unix time in milliseconds.
    fn now_ms(&self) -> u64;
    /// The directory holding logs, sessions and caches, created on first use.
    fn state_dir(&self) -> Option<PathBuf>;
    /// Replace `path` with `text`, readable by the owner only, never leaving half a file.
    fn write_private(&self, path: &Path, text: &str) -> std::io::Result<()>;
    /// Append one line to the log at `path`; never fails the caller.
    fn append_line(&self, path: &str, text: &str);
    /// The `git -c` options that stop a repository from running its own programs for `dir`.
    fn no_repo_programs(&self, dir: &Path) -> Vec<String>;
}

static INSTALLED: OnceLock<Box<dyn Host>> = OnceLock::new();

/// Use `host` for the rest of the process. `false` when one was already chosen (by an earlier
/// call, or by a first use that took the system adapter).
pub fn install(host: Box<dyn Host>) -> bool {
    INSTALLED.set(host).is_ok()
}

/// The host in use: the installed one, else the system adapter.
pub fn get() -> &'static dyn Host {
    &**INSTALLED.get_or_init(|| Box::new(crate::shared::infrastructure::system_host::SystemHost))
}

/// [`Host::env`].
pub fn env(name: &str) -> Option<String> {
    get().env(name)
}

/// [`Host::now_ms`].
pub fn now_ms() -> u64 {
    get().now_ms()
}

/// [`Host::state_dir`].
pub fn state_dir() -> Option<PathBuf> {
    get().state_dir()
}

/// `var` if set, else `file` inside the state directory.
pub fn path_from_env(var: &str, file: &str) -> Option<String> {
    env(var).or_else(|| Some(state_dir()?.join(file).to_string_lossy().into_owned()))
}

/// [`Host::write_private`].
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    get().write_private(path, text)
}

/// [`Host::append_line`].
pub fn append_line(path: &str, text: &str) {
    get().append_line(path, text);
}

/// [`Host::no_repo_programs`].
pub fn no_repo_programs(dir: &Path) -> Vec<String> {
    get().no_repo_programs(dir)
}
