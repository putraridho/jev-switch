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

/// The entry's text: `Name=Flashboard`, `Exec=<quoted exec> --hidden`. Pure.
pub fn desktop_entry(exec: &str) -> String {
    let _ = exec;
    String::new()
}

/// Makes `<config_home>/autostart/dev.jev.flashboard.desktop` match `enabled` (atomic, 0600;
/// rewrites when `Exec` differs).
pub fn reconcile(enabled: bool, exec: &Path, config_home: &Path) -> Result<(), AutostartError> {
    let _ = (enabled, exec, config_home);
    Err(AutostartError::Write)
}
