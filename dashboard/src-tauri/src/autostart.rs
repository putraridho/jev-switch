//! The XDG autostart entry: the one file written outside the state directory (DESIGN 9.13).
//! T15 writes the bodies.

use std::path::Path;

/// Why the autostart entry could not be made to match the setting.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum AutostartError {
    /// The binary's path is not valid UTF-8 or not an existing file; nothing was written.
    InvalidExec,
    /// The entry could not be written or removed.
    Write,
}

const FILE_NAME: &str = "dev.jev.flashboard.desktop";

/// The entry's text: `Name=Flashboard`, `Exec=<quoted exec> --hidden`. Pure.
pub fn desktop_entry(exec: &str) -> String {
    let mut quoted = String::with_capacity(exec.len().saturating_add(2));
    quoted.push('"');
    for c in exec.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=Flashboard\nExec={quoted} --hidden\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// Makes `<config_home>/autostart/dev.jev.flashboard.desktop` match `enabled` (atomic, 0600;
/// rewrites when `Exec` differs).
pub fn reconcile(enabled: bool, exec: &Path, config_home: &Path) -> Result<(), AutostartError> {
    let dir = config_home.join("autostart");
    let path = dir.join(FILE_NAME);
    if !enabled {
        return match jev_usage_io::fsutil::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(_) => Err(AutostartError::Write),
        };
    }
    let exec_str = exec.to_str().ok_or(AutostartError::InvalidExec)?;
    // A line break in the path would start a new line of the entry.
    if exec_str.contains(['\n', '\r']) {
        return Err(AutostartError::InvalidExec);
    }
    if !exec.is_file() {
        return Err(AutostartError::InvalidExec);
    }
    let text = desktop_entry(exec_str);
    if jev_usage_io::fsutil::ensure_private_dir(&dir).is_err() {
        return Err(AutostartError::Write);
    }
    match jev_usage_io::fsutil::write_private_atomic(&path, text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(_) => Err(AutostartError::Write),
    }
}

/// The user's config home: `XDG_CONFIG_HOME`, else `~/.config`.
fn config_home() -> Option<std::path::PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => Some(std::path::PathBuf::from(dir)),
        _ => std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| std::path::PathBuf::from(home).join(".config")),
    }
}

/// Makes the autostart entry match `enabled` for the running binary and reports a failure on
/// the diagnostic channel (the app keeps running either way).
pub fn apply(enabled: bool) {
    let result = match (std::env::current_exe(), config_home()) {
        (Ok(exec), Some(config)) => reconcile(enabled, &exec, &config),
        _ => Err(AutostartError::InvalidExec),
    };
    if result.is_err() {
        jev_usage_io::diag::emit(jev_usage_io::diag::DiagEvent::AutostartFailed);
    }
}
