//! Is a provider CLI really installed, as opposed to a lazy mise launcher that would install it?

use std::path::{Path, PathBuf};

/// A lazy mise launcher: a command-wrapper or a shim. Running one for a tool that is not
/// installed yet STARTS A LONG INSTALL (it happened with `grok` and `agy`), so a probe
/// must never execute one unless mise already has that tool installed.
pub fn is_mise_wrapper(path: &Path) -> bool {
    let parts: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    parts.iter().any(|c| *c == "command-wrappers")
        || parts.windows(2).any(|w| w[0] == "mise" && w[1] == "shims")
}

/// The pure decision: `bin` is really installed when some candidate on PATH is a real
/// binary, or when every candidate is a lazy launcher but mise already has the tool.
pub fn really_installed(candidates: &[PathBuf], mise_has_tool: bool) -> bool {
    candidates.iter().any(|p| !is_mise_wrapper(p)) || (!candidates.is_empty() && mise_has_tool)
}

/// mise's data directory: `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`, else
/// `~/.local/share/mise`.
pub fn mise_data_dir(mise_data_dir: Option<String>, xdg_data_home: Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
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
pub fn mise_installs_binary(versions: &[PathBuf], bin: &str, is_exec: &dyn Fn(&Path) -> bool) -> bool {
    versions.iter().any(|version| [version.join(bin), version.join("bin").join(bin)].iter().any(|exe| is_exec(exe)))
}
