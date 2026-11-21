//! The doors every use case has to the machine it runs on: settings (environment and key
//! file), the clock, the state directory, private file writes and git's safety options.
//!
//! Owns the port ([`Host`]): the plan's `Env`, `Clock` and `StateDir` ports in one trait
//! (DDD.md P3). Must not touch the environment, the clock or the disk itself. Use cases receive
//! a `Host` through their constructor or as a parameter; the composition roots pass the real
//! adapter, [`crate::shared::infrastructure::system_host::SystemHost`], and tests pass their own.

use std::path::{Path, PathBuf};

/// What the machine can tell or do for a use case.
pub trait Host: Send + Sync {
    /// A setting: the environment first, then the key file; empty counts as unset.
    fn env(&self, name: &str) -> Option<String>;
    /// The process HOME, without consulting the settings file.
    fn home(&self) -> Option<String>;
    /// The current process identifier.
    fn process_id(&self) -> u32;
    /// Unix time in milliseconds.
    fn now_ms(&self) -> u64;
    /// The directory holding logs, sessions and caches, created on first use.
    fn state_dir(&self) -> Option<PathBuf>;
    /// Read a UTF-8 text file.
    fn read_text(&self, path: &Path) -> std::io::Result<String>;
    /// Replace `path` with `text`, readable by the owner only, never leaving half a file.
    fn write_private(&self, path: &Path, text: &str) -> std::io::Result<()>;
    /// Append one line to the log at `path`; never fails the caller.
    fn append_line(&self, path: &str, text: &str);
    /// The `git -c` options that stop a repository from running its own programs for `dir`.
    fn no_repo_programs(&self, dir: &Path) -> Vec<String>;
}
