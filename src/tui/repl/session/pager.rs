//! Showing long output through `$PAGER`.

use crate::tui::views::status_facts as views;

/// Show `text` through `$PAGER` (default `less -R`). `false` when no pager could run, so
/// the caller prints it itself.
pub fn page(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command as Process, Stdio};
    let (program, args) = views::pager_command(crate::shared::infrastructure::env_file::lookup("PAGER").as_deref());
    let Ok(mut child) = Process::new(program).args(args).stdin(Stdio::piped()).spawn() else { return false };
    if let Some(mut stdin) = child.stdin.take() {
        // The pager may quit before reading everything; that is not an error.
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    true
}
