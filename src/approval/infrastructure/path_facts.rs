//! The real [`PathFacts`]: this machine's file system.

use std::path::{Path, PathBuf};

use crate::approval::domain::grantable::PathFacts;

pub struct SystemPathFacts;

impl PathFacts for SystemPathFacts {
    fn is_symlink(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    }

    fn real_path(&self, path: &Path) -> Option<PathBuf> {
        path.canonicalize().ok()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}
