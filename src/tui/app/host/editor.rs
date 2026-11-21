//! The editor: `$VISUAL` or `$EDITOR` for `/edit` and `/memory`, and composing a message in it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The editor to open: `$VISUAL`, else `$EDITOR`, else `vi`, split into program and
/// arguments (`code --wait` works).
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> (String, Vec<String>) {
    let chosen = [visual, editor].into_iter().flatten().map(str::trim).find(|s| !s.is_empty()).unwrap_or("vi");
    let mut words = chosen.split_whitespace().map(String::from);
    let program = words.next().unwrap_or_else(|| "vi".to_string());
    (program, words.collect())
}

/// Open `path` in the editor and wait for it to close. The error is for the user.
pub fn edit_file(path: &Path) -> Result<(), String> {
    let (program, args) = editor_command(crate::shared::infrastructure::env_file::lookup("VISUAL").as_deref(), crate::shared::infrastructure::env_file::lookup("EDITOR").as_deref());
    let status = Command::new(&program).args(&args).arg(path).status().map_err(|e| format!("cannot start the editor `{program}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the editor `{program}` exited with {status}"))
    }
}

/// A private scratch file for composing a message, with `initial` in it.
pub fn scratch_file(state_dir: Option<&Path>, initial: &str) -> std::io::Result<PathBuf> {
    let dir = state_dir.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("message-{}.md", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&path)?.write_all(initial.as_bytes())?;
    Ok(path)
}

/// Compose a message in the editor: the text written, trimmed; `Ok(None)` when it was left
/// empty. The scratch file is removed either way.
pub fn compose(state_dir: Option<&Path>) -> Result<Option<String>, String> {
    let path = scratch_file(state_dir, "").map_err(|e| format!("cannot make a scratch file: {e}"))?;
    let edited = edit_file(&path);
    let text = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    edited?;
    let text = text.map_err(|e| format!("cannot read what was written: {e}"))?;
    let text = text.trim();
    Ok((!text.is_empty()).then(|| text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_comes_from_visual_then_editor_then_vi() {
        assert_eq!(editor_command(Some("code --wait"), Some("nano")), ("code".to_string(), vec!["--wait".to_string()]));
        assert_eq!(editor_command(Some("  "), Some("nano")), ("nano".to_string(), vec![]));
        assert_eq!(editor_command(None, None), ("vi".to_string(), vec![]));
    }

    #[test]
    fn a_scratch_file_is_private() {
        let dir = std::env::temp_dir().join(format!("jev-host-{}", std::process::id()));
        let path = scratch_file(Some(&dir), "hi").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
