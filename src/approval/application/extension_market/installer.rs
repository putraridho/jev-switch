//! Installing, updating and removing: stage a verified copy, swap it in, record where it came from.

use std::path::Path;

use super::listing::{listing_of, State};
use super::manifest::{checked_path, read_manifest, reject_links};
use super::pristine::is_pristine_copy;
use super::{extensions_dir, fail, load_registry, save_registry, MarketError, Record, MANIFEST};
use crate::shared::domain::sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub files: usize,
    /// The version it replaced, when this was an update.
    pub replaced: Option<String>,
}

/// Install (or replace) the extension in the folder `source` under `user`'s extensions.
pub fn install(user: &Path, source: &Path) -> Result<Installed, MarketError> {
    let source = source.canonicalize().map_err(|e| MarketError(format!("{}: {e}", source.display())))?;
    let manifest = read_manifest(&source)?;
    if manifest.files.is_empty() {
        return fail(format!("{} ships no files", manifest.name));
    }
    // Verify everything before touching the installed copy.
    let mut bodies = Vec::new();
    for (rel, expected) in &manifest.files {
        let rel_path = checked_path(rel)?;
        reject_links(&source, &rel_path, rel)?;
        let path = source.join(rel_path);
        let bytes = std::fs::read(&path).map_err(|e| MarketError(format!("{rel}: {e}")))?;
        if sha256::hex(&bytes) != expected.to_ascii_lowercase() {
            return fail(format!("checksum mismatch for {rel}: the file is not what {} {} was published with; nothing was installed", manifest.name, manifest.version));
        }
        bodies.push((rel.clone(), bytes));
    }
    let target = extensions_dir(user).join(&manifest.name);
    let mut registry = load_registry(user);
    let replaced = registry.extensions.iter().find(|r| r.name == manifest.name).map(|r| r.version.clone());
    // A registry entry identifies a managed copy. Without one, only a pristine copy with
    // the expected manifest can be safely replaced after a lost or unreadable registry.
    let installed_by_jev = replaced.is_some() || is_pristine_copy(&target, &manifest.name);
    if target.exists() && !installed_by_jev {
        return fail(format!("{} exists and was not installed from a package; remove or rename it first", target.display()));
    }
    // Write the new copy beside the installed one and swap them, so a failed write (disk full,
    // permissions) leaves the installed extension as it was.
    let stage = extensions_dir(user).join(format!(".{}.new", manifest.name));
    let backup = extensions_dir(user).join(format!(".{}.old", manifest.name));
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_dir_all(&backup);
    let staged = (|| -> Result<(), MarketError> {
        for (rel, bytes) in &bodies {
            let to = stage.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| MarketError(format!("{}: {e}", parent.display())))?;
            }
            std::fs::write(&to, bytes).map_err(|e| MarketError(format!("{}: {e}", to.display())))?;
        }
        let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| MarketError(e.to_string()))?;
        std::fs::write(stage.join(MANIFEST), manifest_text).map_err(|e| MarketError(e.to_string()))
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let had_old = target.exists();
    if had_old {
        if let Err(e) = std::fs::rename(&target, &backup) {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(MarketError(format!("{}: {e}", target.display())));
        }
    }
    if let Err(e) = std::fs::rename(&stage, &target) {
        if had_old {
            let _ = std::fs::rename(&backup, &target);
        }
        let _ = std::fs::remove_dir_all(&stage);
        return Err(MarketError(format!("{}: {e}", target.display())));
    }
    let _ = std::fs::remove_dir_all(&backup);
    registry.extensions.retain(|r| r.name != manifest.name);
    registry.extensions.push(Record { name: manifest.name.clone(), version: manifest.version.clone(), source });
    registry.extensions.sort_by(|a, b| a.name.cmp(&b.name));
    save_registry(user, &registry)?;
    Ok(Installed { name: manifest.name, version: manifest.version, files: bodies.len(), replaced })
}

/// Fetch the extension again from where it was installed from. `Ok(None)` when that source
/// still has the version that is installed.
pub fn update(user: &Path, name: &str) -> Result<Option<Installed>, MarketError> {
    let registry = load_registry(user);
    let Some(record) = registry.extensions.iter().find(|r| r.name == name) else {
        return fail(format!("{name} is not installed"));
    };
    let offered = read_manifest(&record.source).map_err(|e| MarketError(format!("cannot update {name}: {e}")))?;
    if offered.name != name {
        return fail(format!("cannot update {name}: its source now ships `{}`", offered.name));
    }
    if offered.version == record.version && listing_of(user, record).state == State::Verified {
        return Ok(None);
    }
    install(user, &record.source).map(Some)
}

/// Delete an installed extension and forget it.
pub fn remove(user: &Path, name: &str) -> Result<(), MarketError> {
    let mut registry = load_registry(user);
    if !registry.extensions.iter().any(|r| r.name == name) {
        return fail(format!("{name} is not installed"));
    }
    let target = extensions_dir(user).join(name);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| MarketError(format!("{}: {e}", target.display())))?;
    }
    registry.extensions.retain(|r| r.name != name);
    save_registry(user, &registry)
}
