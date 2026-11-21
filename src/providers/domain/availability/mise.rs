//! Is a provider CLI really installed, as opposed to a lazy mise launcher that would install it?

use std::path::{Path, PathBuf};

use super::host::{find_on_path, home, is_executable_file, path_dirs};

/// A lazy mise launcher: a command-wrapper or a shim. Running one for a tool that is not
/// installed yet STARTS A LONG INSTALL (it happened with `grok` and `agy`), so a probe
/// must never execute one unless mise already has that tool installed.
pub(super) fn is_mise_wrapper(path: &Path) -> bool {
    let parts: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    parts.iter().any(|c| *c == "command-wrappers")
        || parts.windows(2).any(|w| w[0] == "mise" && w[1] == "shims")
}

/// The pure decision: `bin` is really installed when some candidate on PATH is a real
/// binary, or when every candidate is a lazy launcher but mise already has the tool.
pub(super) fn really_installed(candidates: &[PathBuf], mise_has_tool: bool) -> bool {
    candidates.iter().any(|p| !is_mise_wrapper(p)) || (!candidates.is_empty() && mise_has_tool)
}

/// Whether running `bin` is safe for a probe: a real binary, never a lazy launcher that
/// would install it. Use this, not [`on_path`], before executing any provider CLI.
pub(crate) fn installed_for_real(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable_file(Path::new(bin)) && !is_mise_wrapper(Path::new(bin));
    }
    let candidates = find_on_path(bin, &path_dirs(), &is_executable_file);
    let data = mise_data_dir(std::env::var("MISE_DATA_DIR").ok(), std::env::var("XDG_DATA_HOME").ok(), home());
    really_installed(&candidates, data.is_some_and(|d| mise_installs_binary(&d, bin)))
}

/// mise's data directory: `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`, else
/// `~/.local/share/mise`.
pub(super) fn mise_data_dir(mise_data_dir: Option<String>, xdg_data_home: Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
    mise_data_dir
        .map(PathBuf::from)
        .or_else(|| xdg_data_home.map(|x| PathBuf::from(x).join("mise")))
        .or_else(|| home.map(|h| h.join(".local/share/mise")))
}

/// True when some mise install holds an executable file called `bin`: `installs/<tool>/<version>/<bin>`
/// or `installs/<tool>/<version>/bin/<bin>`, whatever the tool directory is called (a tool
/// installed through a backend is `npm-google-gemini-cli`, its binary `gemini`).
///
/// Found from the filesystem alone, and a directory is not enough: mise creates an EMPTY
/// version directory when an install is started, and Omarchy's account wrapper runs `mise
/// install` (or `mise use -g`) whenever it finds no real binary, so an empty or half-done
/// install must count as "not installed". Nothing is executed to find out.
pub(super) fn mise_installs_binary(data_dir: &Path, bin: &str) -> bool {
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
    };
    subdirs(&data_dir.join("installs"))
        .iter()
        .flat_map(|tool| subdirs(tool))
        .any(|version| [version.join(bin), version.join("bin").join(bin)].iter().any(|exe| is_executable_file(exe)))
}
