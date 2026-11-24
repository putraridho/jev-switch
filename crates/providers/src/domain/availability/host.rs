//! Pure PATH search used by the provider probes.

use std::path::{Path, PathBuf};

/// Every executable called `name` in `dirs`, in PATH order. `is_exec` is injected so
/// the search is testable without a filesystem.
///
/// On Windows a program is `name.exe` or `name.cmd`, so each directory is searched for those
/// names too.
pub fn find_on_path(name: &str, dirs: &[PathBuf], is_exec: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let names = jev_shared::domain::platform::executable_names(name);
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).filter(|p| is_exec(p)).collect()
}
