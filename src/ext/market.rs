//! Installing, updating and removing extensions (commands, skills, recipes, themes, mods),
//! with a checksum for every file.
//!
//! An extension is a folder with a `jev-extension.json` manifest next to the files it ships:
//!
//! ```json
//! { "name": "review-kit", "version": "1.2.0", "description": "...",
//!   "files": { "commands/review-pr.md": "<sha-256 hex>", "recipes/fix-tests.md": "<sha-256 hex>" } }
//! ```
//!
//! `pack` writes the manifest from the files; `install` refuses a source whose files do not
//! match it, copies only what the manifest lists, and remembers where it came from so `update`
//! can fetch it again; `list` re-hashes what is installed and says when something was edited.
//! Sources are folders on disk. Hooks are not an extension file type: a hook runs shell
//! commands, so it can only be written by the person themselves.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::library::valid_name;
use super::sha256;

pub const MANIFEST: &str = "jev-extension.json";
const REGISTRY: &str = "extensions.json";
/// The folders an extension may put files in.
const ALLOWED_DIRS: [&str; 5] = ["commands", "skills", "recipes", "themes", "mods"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketError(pub String);

impl std::fmt::Display for MarketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MarketError {}

fn fail<T>(message: impl Into<String>) -> Result<T, MarketError> {
    Err(MarketError(message.into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Path inside the extension to the SHA-256 of that file.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    name: String,
    version: String,
    source: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    extensions: Vec<Record>,
}

fn extensions_dir(user: &Path) -> PathBuf {
    user.join("extensions")
}

/// The folders of the installed extensions, in name order.
pub fn installed_dirs(user: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(extensions_dir(user)) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

fn load_registry(user: &Path) -> Registry {
    std::fs::read_to_string(user.join(REGISTRY)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_registry(user: &Path, registry: &Registry) -> Result<(), MarketError> {
    let text = serde_json::to_string_pretty(registry).map_err(|e| MarketError(e.to_string()))?;
    std::fs::create_dir_all(user).map_err(|e| MarketError(format!("{}: {e}", user.display())))?;
    crate::handoff::write_atomic(&user.join(REGISTRY), &text).map_err(|e| MarketError(format!("cannot save the extension list: {e}")))
}

/// A manifest path that stays inside the extension and in a folder extensions may use.
fn checked_path(rel: &str) -> Result<PathBuf, MarketError> {
    let path = Path::new(rel);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_))) && path.components().count() >= 2;
    let top = path.components().next().and_then(|c| c.as_os_str().to_str()).unwrap_or("");
    if !plain || !ALLOWED_DIRS.contains(&top) {
        return fail(format!("`{rel}` is not allowed: files go in {} (no `..`, no absolute paths)", ALLOWED_DIRS.join(", ")));
    }
    Ok(path.to_path_buf())
}

fn read_manifest(dir: &Path) -> Result<Manifest, MarketError> {
    let file = dir.join(MANIFEST);
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

fn hash_file(path: &Path) -> Result<String, MarketError> {
    std::fs::read(path).map(|b| sha256::hex(&b)).map_err(|e| MarketError(format!("{}: {e}", path.display())))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
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
        walk(&dir.join(top), dir, &mut files);
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
        let path = source.join(checked_path(rel)?);
        let bytes = std::fs::read(&path).map_err(|e| MarketError(format!("{rel}: {e}")))?;
        if sha256::hex(&bytes) != expected.to_ascii_lowercase() {
            return fail(format!("checksum mismatch for {rel}: the file is not what {} {} was published with; nothing was installed", manifest.name, manifest.version));
        }
        bodies.push((rel.clone(), bytes));
    }
    let target = extensions_dir(user).join(&manifest.name);
    let mut registry = load_registry(user);
    let replaced = registry.extensions.iter().find(|r| r.name == manifest.name).map(|r| r.version.clone());
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| MarketError(format!("{}: {e}", target.display())))?;
    }
    for (rel, bytes) in &bodies {
        let to = target.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MarketError(format!("{}: {e}", parent.display())))?;
        }
        std::fs::write(&to, bytes).map_err(|e| MarketError(format!("{}: {e}", to.display())))?;
    }
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| MarketError(e.to_string()))?;
    std::fs::write(target.join(MANIFEST), manifest_text).map_err(|e| MarketError(e.to_string()))?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Every file still has the checksum it was installed with.
    Verified,
    /// These files were changed, or are gone, since installing.
    Changed(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub version: String,
    pub source: PathBuf,
    pub state: State,
}

fn listing_of(user: &Path, record: &Record) -> Listing {
    let dir = extensions_dir(user).join(&record.name);
    let state = match read_manifest(&dir) {
        Err(_) => State::Changed(vec![MANIFEST.to_string()]),
        Ok(manifest) => {
            let bad: Vec<String> =
                manifest.files.iter().filter(|(rel, sum)| hash_file(&dir.join(rel)).map_or(true, |now| now != **sum)).map(|(rel, _)| rel.clone()).collect();
            if bad.is_empty() {
                State::Verified
            } else {
                State::Changed(bad)
            }
        }
    };
    Listing { name: record.name.clone(), version: record.version.clone(), source: record.source.clone(), state }
}

/// What is installed, each with its checksum state.
pub fn list(user: &Path) -> Vec<Listing> {
    load_registry(user).extensions.iter().map(|r| listing_of(user, r)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-market-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn package(dir: &Path, version: &str, body: &str) {
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::write(dir.join("commands/hello.md"), body).unwrap();
        pack(dir, "hello-kit", version).unwrap();
    }

    #[test]
    fn pack_install_list_and_remove() {
        let root = temp("flow");
        let (src, user) = (root.join("src"), root.join("cfg"));
        package(&src, "1.0.0", "say hello");
        let manifest = read_manifest(&src).unwrap();
        assert_eq!(manifest.files["commands/hello.md"], sha256::hex(b"say hello"));

        let done = install(&user, &src).unwrap();
        assert_eq!(done, Installed { name: "hello-kit".into(), version: "1.0.0".into(), files: 1, replaced: None });
        assert!(user.join("extensions/hello-kit/commands/hello.md").exists());
        assert_eq!(installed_dirs(&user), vec![user.join("extensions/hello-kit")]);
        let listed = list(&user);
        assert_eq!((listed.len(), &listed[0].state), (1, &State::Verified));

        remove(&user, "hello-kit").unwrap();
        assert!(list(&user).is_empty() && installed_dirs(&user).is_empty());
        assert!(remove(&user, "hello-kit").unwrap_err().0.contains("not installed"));
    }

    #[test]
    fn a_file_that_does_not_match_its_checksum_installs_nothing() {
        let root = temp("tamper");
        let (src, user) = (root.join("src"), root.join("cfg"));
        package(&src, "1.0.0", "say hello");
        std::fs::write(src.join("commands/hello.md"), "say hello; and exfiltrate").unwrap();
        let err = install(&user, &src).unwrap_err().0;
        assert!(err.contains("checksum mismatch for commands/hello.md"), "{err}");
        assert!(installed_dirs(&user).is_empty() && list(&user).is_empty());
    }

    #[test]
    fn manifests_cannot_reach_outside_the_allowed_folders() {
        for bad in ["../evil.md", "/etc/passwd", "hooks/run.md", "commands", "commands/../../x"] {
            assert!(checked_path(bad).is_err(), "{bad}");
        }
        assert!(checked_path("skills/db/SKILL.md").is_ok());
        let root = temp("escape");
        std::fs::write(root.join(MANIFEST), r#"{"name":"x","version":"1","files":{"../escape.md":"00"}}"#).unwrap();
        assert!(install(&root.join("cfg"), &root).unwrap_err().0.contains("not allowed"));
        std::fs::write(root.join(MANIFEST), r#"{"name":"Bad Name","version":"1","files":{}}"#).unwrap();
        assert!(install(&root.join("cfg"), &root).unwrap_err().0.contains("not a usable extension name"));
    }

    #[test]
    fn list_notices_edits_and_update_restores_them() {
        let root = temp("update");
        let (src, user) = (root.join("src"), root.join("cfg"));
        package(&src, "1.0.0", "v1");
        install(&user, &src).unwrap();
        assert_eq!(update(&user, "hello-kit").unwrap(), None, "same version, intact: nothing to do");

        std::fs::write(user.join("extensions/hello-kit/commands/hello.md"), "edited").unwrap();
        assert_eq!(list(&user)[0].state, State::Changed(vec!["commands/hello.md".into()]));
        let healed = update(&user, "hello-kit").unwrap().unwrap();
        assert_eq!(healed.replaced.as_deref(), Some("1.0.0"));
        assert_eq!(list(&user)[0].state, State::Verified);

        package(&src, "1.1.0", "v1.1");
        let up = update(&user, "hello-kit").unwrap().unwrap();
        assert_eq!((up.version.as_str(), up.replaced.as_deref()), ("1.1.0", Some("1.0.0")));
        assert_eq!(std::fs::read_to_string(user.join("extensions/hello-kit/commands/hello.md")).unwrap(), "v1.1");
        assert!(update(&user, "nope").unwrap_err().0.contains("not installed"));
    }

    #[test]
    fn pack_needs_files_and_a_valid_name() {
        let root = temp("pack");
        assert!(pack(&root, "x", "1").unwrap_err().0.contains("nothing to pack"));
        std::fs::create_dir_all(root.join("recipes")).unwrap();
        std::fs::write(root.join("recipes/r.md"), "go").unwrap();
        assert!(pack(&root, "Bad", "1").is_err());
        assert_eq!(pack(&root, "ok", "2").unwrap().files.len(), 1);
    }
}
