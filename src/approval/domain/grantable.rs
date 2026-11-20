//! Which folders and files an "always" may grant, judged from the project, home and temp.

use std::path::{Component, Path, PathBuf};
use crate::approval::domain::refusal::Refusal;

/// Where rules and folder grants are judged from: the project, the home folder and the
/// system's temp folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub project: PathBuf,
    pub home: Option<PathBuf>,
    pub temp: PathBuf,
}

impl Places {
    /// The places of the running process, for the project at `project`.
    pub fn from_env(project: &Path) -> Self {
        Self { project: project.to_path_buf(), home: std::env::var_os("HOME").map(PathBuf::from), temp: std::env::temp_dir() }
    }
}

/// Folder names (lower case) that hold secrets whatever their place, besides every hidden
/// folder (`.ssh`, `.gnupg`, `.aws`, `.config`, `.local/share`, `.password-store`, `.kube`,
/// `.docker`, `.mozilla`, `.claude`, `.git`: all start with a dot).
const SENSITIVE_FOLDERS: &[&str] = &["keyrings", "keychains", "secrets", "credentials"];

/// Top-level folders of the system itself: neither the home nor the temp folder can be one
/// of them or inside one (`HOME=/etc/x`, `TMPDIR=/usr/bin`).
const SYSTEM_TOP: &[&str] = &[
    "bin", "boot", "dev", "etc", "lib", "lib32", "lib64", "libx32", "proc", "sbin", "sys", "usr", "System", "Library", "Applications",
];

/// Top-level folders that hold other people's or the system's things: fine to be inside
/// (`/home/u`, `/var/folders/x/T`), never the temp folder themselves (`TMPDIR=/home`).
/// `/tmp` itself is the usual temp folder and is allowed.
const SHARED_TOP: &[&str] = &["home", "Users", "var", "private", "opt", "srv", "mnt", "media", "run", "root", "Volumes"];

/// Absolute, and made of plain names only (no `.` or `..`).
fn is_plain_absolute(path: &Path) -> bool {
    path.is_absolute() && path.components().skip(1).all(|c| matches!(c, Component::Normal(_)))
}

/// The plain names of a path, as text.
fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Whether `home` (from `HOME`) is a home folder grants may be judged against: absolute, at
/// least two folders deep (`/home/u`; not `/`, `/home` or `/root`) and not a system folder.
/// Otherwise nothing is grantable through it.
fn safe_home(home: &Path) -> bool {
    let names = names(home);
    is_plain_absolute(home) && names.len() >= 2 && !SYSTEM_TOP.contains(&names[0].as_str())
}

/// Whether `temp` (from `TMPDIR`) is a temp folder grants may be judged against: absolute,
/// not the root, not a system folder or inside one, not a shared top folder itself
/// (`/home`, `/var`), and not the home folder or a folder above it (everything below the temp
/// folder is grantable, hidden folders included). Otherwise nothing is grantable through it.
fn safe_temp(temp: &Path, home: Option<&Path>) -> bool {
    let names = names(temp);
    let Some(top) = names.first().map(String::as_str) else { return false };
    is_plain_absolute(temp)
        && !SYSTEM_TOP.contains(&top)
        && (names.len() >= 2 || !SHARED_TOP.contains(&top))
        && !home.is_some_and(|h| h.starts_with(temp))
}

/// Whether `dir` is a folder a permanent grant may name, judged on the text of the path:
/// inside the temp folder (not the temp folder itself), or inside the home folder (not the
/// home folder itself) with no hidden or credentials folder on the way. Everything else
/// (the root, `/usr/bin`, `/etc/ssh`, another user's home, `/opt`, a parent of home) is not.
pub(super) fn grantable_in(dir: &Path, home: Option<&Path>, temp: &Path) -> bool {
    if !is_plain_absolute(dir) {
        return false;
    }
    let home = home.filter(|h| safe_home(h));
    if safe_temp(temp, home) && dir != temp && dir.starts_with(temp) {
        return true;
    }
    let Some(rest) = home.and_then(|h| dir.strip_prefix(h).ok()) else { return false };
    let mut folders = rest.components().peekable();
    folders.peek().is_some()
        && folders.all(|c| match c {
            Component::Normal(name) => {
                let name = name.to_string_lossy().to_ascii_lowercase();
                !name.starts_with('.') && !SENSITIVE_FOLDERS.contains(&name.as_str())
            }
            _ => false,
        })
}

/// The one guard on every permanent grant of a folder: the folder Bash may use
/// (`additionalDirectories`) and the folder a `Read(//dir/**)` rule covers. A symbolic link
/// cannot lead out: when the folder exists, its real path has to pass too.
pub(super) fn grantable_folder(dir: &Path, places: &Places) -> Result<(), Refusal> {
    let lexical = grantable_in(dir, places.home.as_deref(), &places.temp);
    let real = dir.canonicalize().ok().map(|real| {
        let home = places.home.as_deref().map(|h| h.canonicalize().unwrap_or_else(|_| h.to_path_buf()));
        grantable_in(&real, home.as_deref(), &places.temp.canonicalize().unwrap_or_else(|_| places.temp.clone()))
    });
    if lexical && real.unwrap_or(true) {
        Ok(())
    } else {
        Err(Refusal::FolderTooBroad)
    }
}

/// Whether `name` (one path component) is hidden or names a secrets folder.
fn is_sensitive_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    name.starts_with('.') || SENSITIVE_FOLDERS.contains(&name.as_str())
}

/// Whether `file` is inside `project` with no hidden or secrets component on the way, the
/// file's own name included (`.claude/settings.local.json`, `.git/hooks/pre-commit`,
/// `.envrc`): writing those for good could grant or run anything. `None` when `file` is
/// not inside `project`.
fn project_file_in(file: &Path, project: &Path) -> Option<Result<(), Refusal>> {
    let rest = file.strip_prefix(project).ok()?;
    let mut parts = rest.components().peekable();
    let plain = parts.peek().is_some() && parts.all(|c| matches!(c, Component::Normal(name) if !is_sensitive_name(name)));
    Some(if plain { Ok(()) } else { Err(Refusal::SensitiveFile) })
}

/// The one guard on a permanent rule for WRITING a file (absolute `file`): inside the
/// project with no hidden or secrets component, or in a folder [`grantable_folder`] accepts
/// and not hidden itself. A symbolic link cannot lead out: the file must not be one, and
/// the real path of its folder has to pass too.
pub(super) fn grantable_file(file: &Path, places: &Places) -> Result<(), Refusal> {
    if !is_plain_absolute(file) {
        return Err(Refusal::UnsafePath);
    }
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return Err(Refusal::UnsafePath) };
    let judge = |file: &Path, project: &Path| match project_file_in(file, project) {
        Some(verdict) => verdict,
        None if is_sensitive_name(name) => Err(Refusal::SensitiveFile),
        None => file.parent().map_or(Err(Refusal::FolderTooBroad), |dir| grantable_folder(dir, places)),
    };
    judge(file, &places.project)?;
    if std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Refusal::UnsafePath);
    }
    match dir.canonicalize() {
        Ok(real_dir) => {
            let project = places.project.canonicalize().unwrap_or_else(|_| places.project.clone());
            judge(&real_dir.join(name), &project)
        }
        Err(_) => Ok(()),
    }
}
