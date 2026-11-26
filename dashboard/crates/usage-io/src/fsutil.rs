//! Private files (DESIGN 7.5, 8.4): 0700 directories, 0600 files, atomic writes.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// A classified file error without paths or messages.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FsError {
    /// Permission was refused.
    Denied,
    /// The file or a parent directory does not exist.
    Missing,
    /// The path is not usable.
    Invalid,
    /// Any other failure.
    Other,
}

fn classify(error: &io::Error) -> FsError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => FsError::Denied,
        io::ErrorKind::NotFound => FsError::Missing,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData | io::ErrorKind::NotADirectory => {
            FsError::Invalid
        }
        _ => FsError::Other,
    }
}

/// Creates `path` as a directory with mode 0700, and tightens it when it is looser.
pub fn ensure_private_dir(path: &Path) -> Result<(), FsError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| classify(&e))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| classify(&e))
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a 0600 temporary file beside `path`, syncs it, renames it over `path`
/// and syncs the directory.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), FsError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().ok_or(FsError::Invalid)?;
    let mut temp_name = name.to_os_string();
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp-{}-{serial}", std::process::id()));
    let temp = parent.join(temp_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| classify(&e))?;
    let result = (|| {
        file.write_all(bytes).map_err(|e| classify(&e))?;
        file.sync_all().map_err(|e| classify(&e))?;
        fs::rename(&temp, path).map_err(|e| classify(&e))?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| classify(&e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Removes `path`; a missing file is `Ok`.
pub fn remove_file(path: &Path) -> Result<(), FsError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(classify(&e)),
    }
}
