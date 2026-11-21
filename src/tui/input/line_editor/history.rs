//! The history file and the entries kept in the editor.

use std::path::{Path, PathBuf};

use rustyline::history::History;

use super::LineEditor;

/// Lines kept in the history file.
pub const HISTORY_MAX: usize = 2000;

/// `<state dir>/history`.
pub fn history_path(state_dir: &Path) -> PathBuf {
    state_dir.join("history")
}

/// Create the history file readable by its owner only, before anything is written to it:
/// prompts can be private.
pub fn prepare_history_file(path: &Path) {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let _ = opts.open(path);
    restrict(path);
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Add `line` to the history and write it to the file. Blank lines are not kept.
pub fn remember(editor: &mut LineEditor, line: &str, history: Option<&Path>) {
    if line.trim().is_empty() {
        return;
    }
    if editor.add_history_entry(line).unwrap_or(false) {
        if let Some(path) = history {
            let _ = editor.append_history(path);
            restrict(path);
        }
    }
}

/// How many entries the editor remembers (for tests and `/status`-style checks).
pub fn history_len(editor: &LineEditor) -> usize {
    editor.history().len()
}
