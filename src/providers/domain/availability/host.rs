//! Pure PATH search used by the provider probes.

use std::path::{Path, PathBuf};

/// Every executable called `name` in `dirs`, in PATH order. `is_exec` is injected so
/// the search is testable without a filesystem.
pub(crate) fn find_on_path(name: &str, dirs: &[PathBuf], is_exec: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    dirs.iter().map(|d| d.join(name)).filter(|p| is_exec(p)).collect()
}
