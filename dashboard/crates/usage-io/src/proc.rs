//! The one process spawner (DESIGN 8.6): the only module allowed to use the std or tokio
//! process APIs. The types and the two environment lists are frozen; T04 writes the bodies.

use jev_usage_core::usage::Unavailable;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The programs the app can start. There is no variant for `grok`, `agy` or `gemini`, so
/// they cannot be run.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Program {
    /// `codex`.
    Codex,
    /// `claude`.
    Claude,
}

/// What happens to the child's stderr.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Stderr {
    /// Discarded.
    Discard,
    /// Merged into stdout (`2>&1`): only for the two login-status specs.
    Merge,
}

/// One run: a program, constant arguments and input, and its limits.
#[derive(Copy, Clone, Debug)]
pub struct ProcSpec {
    /// The program.
    pub program: Program,
    /// Its arguments: constants, never built from data.
    pub args: &'static [&'static str],
    /// The bytes written to its stdin.
    pub stdin: &'static [u8],
    /// When the run is given up and its process group killed.
    pub deadline: Duration,
    /// What happens to its stderr.
    pub stderr: Stderr,
    /// The most output bytes read in total.
    pub max_output: usize,
}

/// What the line callback wants next.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Flow {
    /// Keep reading.
    Continue,
    /// Close stdin and wait for exit.
    Done,
}

/// How the child ended.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Exit {
    /// Whether it exited with status 0.
    pub success: bool,
}

/// Whether a program can be started without reaching an installer.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Resolved {
    /// An executable is on disk.
    Installed,
    /// No hit on `PATH`, or a launcher with no executable behind it. Nothing is started.
    NotInstalled,
}

/// The variables a child inherits, each only if set in the parent (DESIGN 8.6). No
/// `*_API_KEY`, `*_TOKEN` or proxy variable is on the list.
pub const ENV_ALLOW: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "LANG",
    "LC_ALL",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "MISE_DATA_DIR",
    "MISE_CONFIG_DIR",
];

/// The constants every child gets: a dumb terminal, Claude's own updater off, and mise's
/// installers and self-update off (DESIGN 8.6).
pub const ENV_CONST: &[(&str, &str)] = &[
    ("TERM", "dumb"),
    ("DISABLE_AUTOUPDATER", "1"),
    ("MISE_OFFLINE", "1"),
    ("MISE_AUTO_INSTALL", "false"),
    ("MISE_EXEC_AUTO_INSTALL", "false"),
    ("MISE_NOT_FOUND_AUTO_INSTALL", "false"),
];

/// DESIGN 8.6 resolution, pure over injected directory reads: the first `PATH` hit; a mise
/// launcher counts only when a version directory under `<mise data>/installs/<tool>/` holds
/// the executable `<tool>` or `bin/<tool>`.
pub fn resolve_in(name: &str, path_dirs: &[PathBuf], mise_data: Option<&Path>) -> Resolved {
    let _ = (name, path_dirs, mise_data);
    Resolved::NotInstalled
}

/// Starts programs under the rules of DESIGN 8.6.
#[derive(Debug)]
pub struct ProcRunner {
    _workdir: PathBuf,
}

impl ProcRunner {
    /// Runs `codex` and `claude` by name, through `PATH`, in `workdir`.
    pub fn system(workdir: PathBuf) -> Self {
        Self { _workdir: workdir }
    }

    /// Runs the given files instead of the names on `PATH` (feature `test-support`).
    #[cfg(feature = "test-support")]
    pub fn with_programs(workdir: PathBuf, codex: PathBuf, claude: PathBuf) -> Self {
        let _ = (codex, claude);
        Self { _workdir: workdir }
    }

    /// Whether `program` can be started; runs nothing.
    pub fn resolve(&self, program: Program) -> Resolved {
        let _ = program;
        Resolved::NotInstalled
    }

    /// Spawns under the rules of DESIGN 8.6, writes `stdin`, calls `on_line` per output line.
    pub async fn run(
        &self,
        spec: &ProcSpec,
        on_line: &mut (dyn FnMut(&[u8]) -> Flow + Send),
    ) -> Result<Exit, Unavailable> {
        let _ = (spec, on_line);
        Err(Unavailable::Internal)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    #[test]
    fn environment_lists_equal_the_design() {
        assert_eq!(
            ENV_ALLOW,
            [
                "HOME",
                "USER",
                "LOGNAME",
                "PATH",
                "LANG",
                "LC_ALL",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
                "CODEX_HOME",
                "CLAUDE_CONFIG_DIR",
                "MISE_DATA_DIR",
                "MISE_CONFIG_DIR",
            ]
        );
        assert_eq!(
            ENV_CONST,
            [
                ("TERM", "dumb"),
                ("DISABLE_AUTOUPDATER", "1"),
                ("MISE_OFFLINE", "1"),
                ("MISE_AUTO_INSTALL", "false"),
                ("MISE_EXEC_AUTO_INSTALL", "false"),
                ("MISE_NOT_FOUND_AUTO_INSTALL", "false"),
            ]
        );
    }

    #[test]
    fn no_secret_shaped_name_is_inherited() {
        for name in ENV_ALLOW {
            let upper = name.to_uppercase();
            for banned in ["KEY", "TOKEN", "SECRET", "PROXY", "PASSWORD", "BASE_URL"] {
                assert!(!upper.contains(banned), "{name}");
            }
        }
    }
}
