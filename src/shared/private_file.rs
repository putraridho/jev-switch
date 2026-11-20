//! Owns writing a file privately and atomically: a temporary file with mode 0600,
//! then a rename. This module must not depend on any other crate module.

use std::io::Write as _;
use std::path::Path;

/// Write `text` to `path` through a temp file and a rename, so a crash never leaves half a
/// file; owner-only, because prompts can be private.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    // Never through a symlink planted at the temporary name.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let written = opts.open(&tmp).and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()));
    match written.and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
