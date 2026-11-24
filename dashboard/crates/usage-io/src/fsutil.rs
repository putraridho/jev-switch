//! Private files (DESIGN 7.5, 8.4): 0700 directories, 0600 files, atomic writes. The type
//! is frozen; T06 writes the function bodies.

use std::path::Path;

/// The error of every file helper. `std::io::Error` is classified where it occurs and
/// dropped: no path and no message leaves this module.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FsError {
    /// Permission was refused.
    Denied,
    /// The file or a parent directory does not exist.
    Missing,
    /// The path is not usable (not a directory, not UTF-8 where that is needed).
    Invalid,
    /// Any other failure.
    Other,
}

/// Creates `path` as a directory with mode 0700, and tightens it when it is looser.
pub fn ensure_private_dir(path: &Path) -> Result<(), FsError> {
    let _ = path;
    Err(FsError::Other)
}

/// Writes `bytes` to a 0600 temporary file beside `path`, syncs it, renames it over `path`
/// and syncs the directory.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), FsError> {
    let _ = (path, bytes);
    Err(FsError::Other)
}

/// Removes `path`; a missing file is `Ok`.
pub fn remove_file(path: &Path) -> Result<(), FsError> {
    let _ = path;
    Err(FsError::Other)
}
