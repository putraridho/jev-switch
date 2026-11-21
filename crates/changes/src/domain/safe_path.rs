//! Which paths from git output an undo may join onto the repository root.

use std::path::{Component, Path};

/// A path from git output that is safe to join onto the repository root: relative,
/// no `..`, not inside `.git`, and not mangled by lossy decoding.
pub(super) fn is_safe_rel_path(p: &str) -> bool {
    if p.is_empty() || p.contains('\u{FFFD}') || p.contains('\0') {
        return false;
    }
    let path = Path::new(p);
    !path.is_absolute()
        && path.components().all(|c| match c {
            Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
            _ => false,
        })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_safe_relative_paths_are_accepted() {
        for ok in ["a.txt", "src/c.txt", "dir/.hidden", "a b/c d.rs", ".gitignore", ".github/x.yml"] {
            assert!(is_safe_rel_path(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a/../b", ".git/config", "x/.git/hooks/pre-commit", ".GIT/config", "./a", "a\u{FFFD}b"] {
            assert!(!is_safe_rel_path(bad), "{bad:?}");
        }
    }
}
