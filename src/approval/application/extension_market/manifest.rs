//! The manifest of an extension package: reading it, checking its paths, packing a folder.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use super::{fail, Manifest, MarketError, ALLOWED_DIRS, MANIFEST};
use crate::approval::application::extension_library::valid_name;
use crate::shared::domain::sha256;

/// A manifest path that stays inside the extension and in a folder extensions may use.
pub(super) fn checked_path(rel: &str) -> Result<PathBuf, MarketError> {
    let path = Path::new(rel);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_))) && path.components().count() >= 2;
    let top = path.components().next().and_then(|c| c.as_os_str().to_str()).unwrap_or("");
    if !plain || !ALLOWED_DIRS.contains(&top) {
        return fail(format!("`{rel}` is not allowed: files go in {} (no `..`, no absolute paths)", ALLOWED_DIRS.join(", ")));
    }
    Ok(path.to_path_buf())
}

/// A file in a package must be a plain file in a plain folder: a link (to the file, or in
/// front of it) could bring in a file from outside the package that has the listed checksum.
pub(super) fn reject_links(source: &Path, rel: &Path, shown: &str) -> Result<(), MarketError> {
    let mut at = source.to_path_buf();
    for part in rel.components() {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return fail(format!("{shown} goes through a symbolic link; nothing was installed"));
        }
    }
    Ok(())
}

pub(super) fn read_manifest(dir: &Path) -> Result<Manifest, MarketError> {
    let file = dir.join(MANIFEST);
    // The manifest holds the checksums: it must be the package's own file.
    if std::fs::symlink_metadata(&file).is_ok_and(|m| m.file_type().is_symlink()) {
        return fail(format!("{} is a symbolic link; it must be a file of the package", file.display()));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| MarketError(format!("{}: {e}", file.display())))?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| MarketError(format!("{}: {e}", file.display())))?;
    if !valid_name(&manifest.name) {
        return fail(format!("`{}` is not a usable extension name (lowercase letters, digits, - and _)", manifest.name));
    }
    for rel in manifest.files.keys() {
        checked_path(rel)?;
    }
    Ok(manifest)
}

pub(super) fn hash_file(path: &Path) -> Result<String, MarketError> {
    std::fs::read(path).map(|b| sha256::hex(&b)).map_err(|e| MarketError(format!("{}: {e}", path.display())))
}

pub(super) fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        // `file_type` does not follow links. A link is never packed: it could lead back up
        // (an endless walk) or out of the extension (a private file or folder in a package).
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
}

/// Write `<dir>/jev-extension.json` for the files in the allowed folders of `dir`. The name
/// and version are the ones given, or the existing manifest's.
pub fn pack(dir: &Path, name: &str, version: &str) -> Result<Manifest, MarketError> {
    if !valid_name(name) {
        return fail(format!("`{name}` is not a usable extension name (lowercase letters, digits, - and _)"));
    }
    let mut files = Vec::new();
    for top in ALLOWED_DIRS {
        let folder = dir.join(top);
        // A linked top folder is a link like any other.
        if std::fs::symlink_metadata(&folder).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        walk(&folder, dir, &mut files);
    }
    if files.is_empty() {
        return fail(format!("nothing to pack: no files in {}", ALLOWED_DIRS.join(", ")));
    }
    let mut hashes = BTreeMap::new();
    for rel in files {
        let key = rel.to_string_lossy().replace('\\', "/");
        hashes.insert(key, hash_file(&dir.join(&rel))?);
    }
    let description = read_manifest(dir).map(|m| m.description).unwrap_or_default();
    let manifest = Manifest { name: name.to_string(), version: version.to_string(), description, files: hashes };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| MarketError(e.to_string()))?;
    std::fs::write(dir.join(MANIFEST), text + "\n").map_err(|e| MarketError(format!("cannot write the manifest: {e}")))?;
    Ok(manifest)
}
