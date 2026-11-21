//! Telling whether an installed folder is still exactly what the package shipped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::manifest::{checked_path, hash_file, read_manifest};
use super::{Manifest, MANIFEST};

// Unlike `walk` used for packing, this must refuse links and unreadable entries: anything
// hidden from inspection could be lost when an unregistered folder is replaced.
pub(super) fn contains_only_manifest_files(dir: &Path, base: &Path, allowed: &BTreeSet<PathBuf>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries {
        let Ok(entry) = entry else { return false };
        let Ok(kind) = entry.file_type() else { return false };
        let path = entry.path();
        if kind.is_dir() {
            if !contains_only_manifest_files(&path, base, allowed) {
                return false;
            }
        } else if !kind.is_file() || !(is_system_junk(&entry.file_name()) || path.strip_prefix(base).is_ok_and(|rel| allowed.contains(rel))) {
            return false;
        }
    }
    true
}

/// Files the operating system leaves in folders on its own (Finder's `.DS_Store` and `._`
/// copies, Explorer's `Thumbs.db` and `desktop.ini`): nobody's work, so they never make a
/// folder count as edited.
pub(super) fn is_system_junk(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    matches!(name.as_ref(), ".DS_Store" | "Thumbs.db" | "desktop.ini") || name.starts_with("._")
}

pub(super) fn changed_files(dir: &Path, manifest: &Manifest) -> Vec<String> {
    manifest.files.iter().filter(|(rel, sum)| hash_file(&dir.join(rel)).map_or(true, |now| !now.eq_ignore_ascii_case(sum))).map(|(rel, _)| rel.clone()).collect()
}

pub(super) fn is_pristine_copy(dir: &Path, name: &str) -> bool {
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
        return false;
    }
    let Ok(manifest) = read_manifest(dir) else { return false };
    if manifest.name != name {
        return false;
    }
    let Ok(mut allowed) = manifest.files.keys().map(|rel| checked_path(rel)).collect::<Result<BTreeSet<_>, _>>() else { return false };
    allowed.insert(PathBuf::from(MANIFEST));
    contains_only_manifest_files(dir, dir, &allowed) && changed_files(dir, &manifest).is_empty()
}
