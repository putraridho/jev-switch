//! The one process spawner (DESIGN 8.6): the only module allowed to use the std or tokio
//! process APIs. The types and the two environment lists are frozen; T04 writes the bodies.

use jev_usage_core::usage::Unavailable;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The programs the app can start. There is no variant for `grok` or `gemini`, so they
/// cannot be run.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Program {
    /// `codex`.
    Codex,
    /// `claude`.
    Claude,
    /// `agy`, the Antigravity CLI.
    Agy,
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
    // `agy`'s own background updater; only the exact value `true` switches it off.
    ("AGY_CLI_DISABLE_AUTO_UPDATE", "true"),
    ("MISE_OFFLINE", "1"),
    ("MISE_AUTO_INSTALL", "false"),
    ("MISE_EXEC_AUTO_INSTALL", "false"),
    ("MISE_NOT_FOUND_AUTO_INSTALL", "false"),
];

/// DESIGN 8.6 resolution, pure over injected directory reads: the first `PATH` hit; a mise
/// launcher counts only when a version directory under `<mise data>/installs/<tool>/` holds
/// the executable `<tool>` or `bin/<tool>`.
pub fn resolve_in(name: &str, path_dirs: &[PathBuf], mise_data: Option<&Path>) -> Resolved {
    resolve_tool_in(name, name, path_dirs, mise_data)
}

/// [`resolve_in`] for a binary `name` that mise installs under another tool name.
fn resolve_tool_in(
    name: &str,
    tool: &str,
    path_dirs: &[PathBuf],
    mise_data: Option<&Path>,
) -> Resolved {
    let Some(hit) = path_dirs
        .iter()
        .map(|dir| dir.join(name))
        .find(|p| executable(p))
    else {
        return Resolved::NotInstalled;
    };
    let launcher = hit
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("command-wrappers" | "shims")))
        || hit
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == "mise"))
            .unwrap_or(false);
    if !launcher {
        return Resolved::Installed;
    }
    let installed = mise_data
        .and_then(|data| std::fs::read_dir(data.join("installs").join(tool)).ok())
        .is_some_and(|entries| {
            entries.flatten().any(|entry| {
                let dir = entry.path();
                dir.is_dir()
                    && (executable(&dir.join(name)) || executable(&dir.join("bin").join(name)))
            })
        });
    if installed {
        Resolved::Installed
    } else {
        Resolved::NotInstalled
    }
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

struct GroupGuard(Option<rustix::process::Pid>);
impl Drop for GroupGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

/// Starts programs under the rules of DESIGN 8.6.
#[derive(Debug)]
pub struct ProcRunner {
    workdir: PathBuf,
    codex: Option<PathBuf>,
    claude: Option<PathBuf>,
    agy: Option<PathBuf>,
}

impl ProcRunner {
    /// Runs `codex` and `claude` by name, through `PATH`, in `workdir`.
    pub fn system(workdir: PathBuf) -> Self {
        Self {
            workdir,
            codex: None,
            claude: None,
            agy: None,
        }
    }

    /// Runs the given files instead of the names on `PATH` (feature `test-support`).
    #[cfg(feature = "test-support")]
    pub fn with_programs(workdir: PathBuf, codex: PathBuf, claude: PathBuf) -> Self {
        // Never the real `agy` from `PATH`: without `with_agy` it counts as not installed.
        let agy = Some(workdir.join("agy-not-configured"));
        Self {
            agy,
            workdir,
            codex: Some(codex),
            claude: Some(claude),
        }
    }

    /// Runs the given file instead of `agy` (feature `test-support`).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn with_agy(mut self, agy: PathBuf) -> Self {
        self.agy = Some(agy);
        self
    }

    /// Whether `program` can be started; runs nothing.
    pub fn resolve(&self, program: Program) -> Resolved {
        let path = match program {
            Program::Codex => &self.codex,
            Program::Claude => &self.claude,
            Program::Agy => &self.agy,
        };
        let name = match program {
            Program::Codex => "codex",
            Program::Claude => "claude",
            Program::Agy => "agy",
        };
        let dirs = path.as_ref().and_then(|p| p.parent()).map_or_else(
            || {
                std::env::var_os("PATH")
                    .map_or_else(Vec::new, |p| std::env::split_paths(&p).collect())
            },
            |p| vec![p.to_path_buf()],
        );
        if let Some(path) = path {
            if path.file_name().is_some_and(|n| n != name) {
                return if executable(path) {
                    Resolved::Installed
                } else {
                    Resolved::NotInstalled
                };
            }
        }
        let data = std::env::var_os("MISE_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(|p| PathBuf::from(p).join("mise")))
            .or_else(|| {
                std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share/mise"))
            });
        // mise installs the `agy` binary under the tool name `antigravity-cli`.
        let tool = match program {
            Program::Agy => "antigravity-cli",
            Program::Codex | Program::Claude => name,
        };
        resolve_tool_in(name, tool, &dirs, data.as_deref())
    }

    /// Spawns under the rules of DESIGN 8.6, writes `stdin`, calls `on_line` per output line.
    pub async fn run(
        &self,
        spec: &ProcSpec,
        on_line: &mut (dyn FnMut(&[u8]) -> Flow + Send),
    ) -> Result<Exit, Unavailable> {
        use std::process::Stdio;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use zeroize::{Zeroize, Zeroizing};
        if spec.stderr == Stderr::Merge
            && !matches!(
                (spec.program, spec.args),
                (Program::Codex, ["login", "status"]) | (Program::Claude, ["auth", "status"])
            )
        {
            return Err(Unavailable::Protocol);
        }
        if self.resolve(spec.program) == Resolved::NotInstalled {
            return Err(Unavailable::NotInstalled);
        }
        let path = match spec.program {
            Program::Codex => self.codex.as_deref().unwrap_or(Path::new("codex")),
            Program::Claude => self.claude.as_deref().unwrap_or(Path::new("claude")),
            Program::Agy => self.agy.as_deref().unwrap_or(Path::new("agy")),
        };
        let mut command = tokio::process::Command::new(path);
        command
            .args(spec.args)
            .current_dir(&self.workdir)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if spec.stderr == Stderr::Merge {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .process_group(0);
        for key in ENV_ALLOW {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        for (key, value) in ENV_CONST {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(|_| Unavailable::NotInstalled)?;
        let mut guard = GroupGuard(
            child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .and_then(rustix::process::Pid::from_raw),
        );
        let operation = async {
            let mut stdin = child.stdin.take().ok_or(Unavailable::Protocol)?;
            stdin
                .write_all(spec.stdin)
                .await
                .map_err(|_| Unavailable::Protocol)?;
            let mut stdout = child.stdout.take().ok_or(Unavailable::Protocol)?;
            let mut stderr = child.stderr.take();
            let mut buffer = Zeroizing::new(vec![0_u8; 8192]);
            let mut err_buffer = Zeroizing::new(vec![0_u8; 8192]);
            let mut line = Zeroizing::new(Vec::new());
            let mut total = 0_usize;
            let mut out_open = true;
            let mut err_open = stderr.is_some();
            while out_open || err_open {
                let (count, is_out) = tokio::select! {
                    result = stdout.read(&mut buffer), if out_open => (result.map_err(|_| Unavailable::Protocol)?, true),
                    result = async { match stderr.as_mut() { Some(stream) => stream.read(&mut err_buffer).await, None => Ok(0) } }, if err_open => (result.map_err(|_| Unavailable::Protocol)?, false),
                };
                if count == 0 {
                    if is_out {
                        out_open = false;
                    } else {
                        err_open = false;
                    }
                    continue;
                }
                total = total.checked_add(count).ok_or(Unavailable::Protocol)?;
                if total > spec.max_output {
                    return Err(Unavailable::Protocol);
                }
                let bytes = if is_out { &buffer } else { &err_buffer };
                for byte in bytes.iter().take(count) {
                    line.push(*byte);
                    if line.len() > 1024 * 1024 {
                        return Err(Unavailable::Protocol);
                    }
                    if *byte == b'\n' {
                        if on_line(&line) == Flow::Done {
                            drop(stdin);
                            return child
                                .wait()
                                .await
                                .map(|s| Exit {
                                    success: s.success(),
                                })
                                .map_err(|_| Unavailable::Protocol);
                        }
                        line.zeroize();
                    }
                }
            }
            if !line.is_empty() {
                on_line(&line);
            }
            drop(stdin);
            child
                .wait()
                .await
                .map(|s| Exit {
                    success: s.success(),
                })
                .map_err(|_| Unavailable::Protocol)
        };
        let result = tokio::time::timeout(spec.deadline, operation)
            .await
            .map_err(|_| Unavailable::Timeout)?;
        if result.is_ok() {
            guard.0 = None;
        }
        result
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
                ("AGY_CLI_DISABLE_AUTO_UPDATE", "true"),
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
