//! What one turn changed in the working tree: a summary, the diff, and an undo.
//!
//! The tree is captured as a git *tree object* before and after the turn, through a
//! temporary index (a copy of the real one, so git's stat cache keeps it fast), which
//! means the user's index, branch and working tree are never touched. Untracked files
//! that are not ignored are included; ignored ones never are, so an undo cannot reach
//! them. Everything here is best effort: no git, not a repository or any git failure
//! gives `None`, and a turn never fails because of it.
//!
//! Layout: the parsers and the undo plan are pure and tested on strings; the few
//! functions that run `git` or touch files are thin and tested in real temp repos.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One file the turn changed. Paths are relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The path after the turn (for a rename, the new one).
    pub path: String,
    pub status: ChangeStatus,
    /// Lines added and removed; 0 for binary files.
    pub added: u32,
    pub removed: u32,
    /// For [`ChangeStatus::Renamed`], where the file was before.
    pub old_path: Option<String>,
}

/// The difference between two working-tree snapshots. `files` is never empty in a
/// summary the engine keeps: a turn that changed nothing has no summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSummary {
    pub files: Vec<FileChange>,
    /// Tree ids of the working tree before and after the turn.
    pub before: String,
    pub after: String,
}

impl ChangeSummary {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Total lines (added, removed) over all files.
    pub fn totals(&self) -> (u32, u32) {
        self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
    }
}

// ---------------------------------------------------------------------- parsing (pure)

/// `git diff --name-status -z`: `M\0path\0`, `R087\0old\0new\0`.
fn parse_name_status(raw: &str) -> Vec<(ChangeStatus, String, Option<String>)> {
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let mut out = Vec::new();
    while let Some(code) = parts.next() {
        match code.chars().next() {
            Some('R') | Some('C') => {
                let (Some(old), Some(new)) = (parts.next(), parts.next()) else { break };
                let status = if code.starts_with('R') { ChangeStatus::Renamed } else { ChangeStatus::Added };
                let old = (status == ChangeStatus::Renamed).then(|| old.to_string());
                out.push((status, new.to_string(), old));
            }
            Some(c) => {
                let Some(path) = parts.next() else { break };
                let status = match c {
                    'A' => ChangeStatus::Added,
                    'D' => ChangeStatus::Deleted,
                    _ => ChangeStatus::Modified,
                };
                out.push((status, path.to_string(), None));
            }
            None => {}
        }
    }
    out
}

/// `git diff --numstat -z`: `added\tremoved\tpath\0`, or for a rename
/// `added\tremoved\t\0old\0new\0`. Keyed by the new path; `-` (binary) counts as 0.
fn parse_numstat(raw: &str) -> HashMap<String, (u32, u32)> {
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let mut out = HashMap::new();
    while let Some(record) = parts.next() {
        let mut cols = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (cols.next(), cols.next(), cols.next()) else { continue };
        let path = if path.is_empty() {
            let (Some(_old), Some(new)) = (parts.next(), parts.next()) else { break };
            new
        } else {
            path
        };
        out.insert(path.to_string(), (added.parse().unwrap_or(0), removed.parse().unwrap_or(0)));
    }
    out
}

/// The file list from the two git outputs.
fn build_files(name_status: &str, numstat: &str) -> Vec<FileChange> {
    let counts = parse_numstat(numstat);
    parse_name_status(name_status)
        .into_iter()
        .map(|(status, path, old_path)| {
            let (added, removed) = counts.get(&path).copied().unwrap_or((0, 0));
            FileChange { path, status, added, removed, old_path }
        })
        .collect()
}

// ---------------------------------------------------------------------- git (thin IO)

/// Run `git -C dir args...` (optionally on another index) and return stdout.
fn git(dir: &Path, index: Option<&Path>, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null());
    cmd.env("GIT_OPTIONAL_LOCKS", "0").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
    match index {
        Some(i) => cmd.env("GIT_INDEX_FILE", i),
        None => cmd.env_remove("GIT_INDEX_FILE"),
    };
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn git_text(dir: &Path, index: Option<&Path>, args: &[&str]) -> Option<String> {
    git(dir, index, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A tree id looks like a hex object name (40 chars, or 64 with sha256).
fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn temp_index_path() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("jev-index-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// Where the repository's own index is (it may be relative to `dir`).
fn real_index(dir: &Path) -> Option<PathBuf> {
    let out = git_text(dir, None, &["rev-parse", "--git-path", "index"])?;
    let p = PathBuf::from(out.trim());
    Some(if p.is_absolute() { p } else { dir.join(p) })
}

/// The tree id of the working tree in `dir` right now, untracked files included and
/// ignored ones left out; `None` outside a repository or if git fails. Reads the
/// working tree and writes objects, never the index, branch or files. Blocking: call
/// it from `spawn_blocking`.
pub fn snapshot(dir: &Path) -> Option<String> {
    let real = real_index(dir)?;
    let tmp = temp_index_path();
    // Starting from a copy of the real index keeps git's stat cache, so only files that
    // really changed are hashed again; without one it still works, just slower.
    if real.is_file() {
        let _ = std::fs::copy(&real, &tmp);
    }
    let tree = git(dir, Some(&tmp), &["add", "-A"])
        .and_then(|_| git_text(dir, Some(&tmp), &["write-tree"]))
        .map(|t| t.trim().to_string())
        .filter(|t| is_object_id(t));
    let _ = std::fs::remove_file(&tmp);
    let mut lock = tmp.into_os_string();
    lock.push(".lock");
    let _ = std::fs::remove_file(lock);
    tree
}

/// What changed between two snapshots. `Some` with no files when the trees are equal.
pub fn summarize(dir: &Path, before: &str, after: &str) -> Option<ChangeSummary> {
    if before == after {
        return Some(ChangeSummary { files: Vec::new(), before: before.into(), after: after.into() });
    }
    let diff = |mode: &str| git_text(dir, None, &["diff", mode, "-z", "-M", "--no-ext-diff", before, after]);
    let files = build_files(&diff("--name-status")?, &diff("--numstat")?);
    Some(ChangeSummary { files, before: before.into(), after: after.into() })
}

/// `git diff before after [-- path]` as plain text (no colour; the UI colours it).
pub fn diff_text(dir: &Path, before: &str, after: &str, path: Option<&str>) -> Option<String> {
    let mut args = vec!["--literal-pathspecs", "diff", "--no-color", "--no-ext-diff", "-M", before, after];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    git_text(dir, None, &args)
}

fn toplevel(dir: &Path) -> Option<PathBuf> {
    let out = git_text(dir, None, &["rev-parse", "--show-toplevel"])?;
    let out = out.trim_end_matches(['\n', '\r']);
    (!out.is_empty()).then(|| PathBuf::from(out))
}

// ---------------------------------------------------------------------- undo

/// What an undo did, by repository-relative path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoReport {
    /// Modified files put back to their old content.
    pub restored: Vec<String>,
    /// Files the turn added, now deleted.
    pub removed: Vec<String>,
    /// Files the turn deleted (or renamed away), now back.
    pub recreated: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoError {
    /// There is no previous turn, or it changed nothing, or it was already undone.
    NothingToUndo,
    /// These files differ from how the turn left them: the user or a later turn edited
    /// them again. Nothing was touched.
    ChangedSince(Vec<String>),
    /// git named a path an undo must not touch (outside the repository, or `.git`).
    UnsafePath(String),
    /// git or the file system failed.
    Failed(String),
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToUndo => f.write_str("there is nothing to undo (no previous turn changed any files)"),
            Self::ChangedSince(files) => write!(
                f,
                "not undone: {} changed since that turn, so undoing would lose those edits: {}",
                if files.len() == 1 { "a file was" } else { "files were" },
                files.join(", ")
            ),
            Self::UnsafePath(p) => write!(f, "not undone: refusing to touch `{p}`"),
            Self::Failed(m) => write!(f, "undo failed: {m}"),
        }
    }
}

impl std::error::Error for UndoError {}

/// One thing an undo does to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// The turn created this path.
    Delete(String),
    /// Put the path back to its `before` content; `existed` is whether the turn deleted it.
    Restore { path: String, recreated: bool },
}

impl Step {
    fn path(&self) -> &str {
        match self {
            Self::Delete(p) | Self::Restore { path: p, .. } => p,
        }
    }
}

/// A path from git output that is safe to join onto the repository root: relative,
/// no `..`, not inside `.git`, and not mangled by lossy decoding.
fn is_safe_rel_path(p: &str) -> bool {
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

/// What to do to undo `files`; refuses on the first unsafe path, before anything runs.
fn plan_undo(files: &[FileChange]) -> Result<Vec<Step>, UndoError> {
    let mut steps = Vec::new();
    for f in files {
        let paths = std::iter::once(&f.path).chain(f.old_path.as_ref());
        if let Some(bad) = paths.into_iter().find(|p| !is_safe_rel_path(p)) {
            return Err(UndoError::UnsafePath(bad.clone()));
        }
        match (f.status, &f.old_path) {
            (ChangeStatus::Added, _) => steps.push(Step::Delete(f.path.clone())),
            (ChangeStatus::Modified, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: false }),
            (ChangeStatus::Deleted, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: true }),
            (ChangeStatus::Renamed, old) => {
                steps.push(Step::Delete(f.path.clone()));
                if let Some(old) = old {
                    steps.push(Step::Restore { path: old.clone(), recreated: true });
                }
            }
        }
    }
    if steps.is_empty() {
        return Err(UndoError::NothingToUndo);
    }
    Ok(steps)
}

/// A file mode from `git ls-tree`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File { executable: bool },
    Symlink,
}

/// `git ls-tree -z` lines: `<mode> SP <type> SP <hash> TAB <path>\0`, as path -> (kind, hash).
/// Submodule entries are left out (an undo cannot rebuild them).
fn parse_ls_tree(raw: &str) -> HashMap<String, (Kind, String)> {
    let mut out = HashMap::new();
    for entry in raw.split('\0').filter(|s| !s.is_empty()) {
        let Some((meta, path)) = entry.split_once('\t') else { continue };
        let mut cols = meta.split(' ');
        let (Some(mode), Some(_type), Some(hash)) = (cols.next(), cols.next(), cols.next()) else { continue };
        let kind = match mode {
            "100755" => Kind::File { executable: true },
            "100644" | "100664" => Kind::File { executable: false },
            "120000" => Kind::Symlink,
            _ => continue,
        };
        out.insert(path.to_string(), (kind, hash.to_string()));
    }
    out
}

/// True when `target` really lies inside `root_canon`, looking through symlinks: the
/// deepest existing ancestor is resolved, so a directory that is a link pointing out
/// of the repository is refused.
fn inside_root(root_canon: &Path, target: &Path) -> bool {
    let mut probe = target.parent();
    while let Some(p) = probe {
        if let Ok(real) = std::fs::canonicalize(p) {
            return real.starts_with(root_canon);
        }
        probe = p.parent();
    }
    false
}

/// What an undo will write for one path, read from git before anything is touched.
struct Blob {
    kind: Kind,
    bytes: Vec<u8>,
}

fn io_err(what: &str, path: &str, e: std::io::Error) -> UndoError {
    UndoError::Failed(format!("{what} {path}: {e}"))
}

/// Undo what `summary` records, in the repository `dir` belongs to. Blocking.
///
/// Safe by construction: nothing is touched unless every file the turn changed still
/// matches the tree the turn left behind, every path is inside the repository, and all
/// old contents were read successfully first. Only the paths in the summary are written.
pub fn undo(dir: &Path, summary: &ChangeSummary) -> Result<UndoReport, UndoError> {
    let steps = plan_undo(&summary.files)?;
    let root = toplevel(dir).ok_or_else(|| UndoError::Failed("not inside a git repository".into()))?;
    let root_canon = std::fs::canonicalize(&root).map_err(|e| io_err("cannot resolve", &root.to_string_lossy(), e))?;

    // 1. Has anything the turn touched been edited since? Compare against the tree the
    // turn left, not against `before`: only a difference from `after` is someone's edit.
    let now = snapshot(dir).ok_or_else(|| UndoError::Failed("could not read the working tree".into()))?;
    let differing = git_text(&root, None, &["diff", "--name-only", "-z", "--no-renames", &summary.after, &now])
        .ok_or_else(|| UndoError::Failed("could not compare the working tree".into()))?;
    let differing: HashSet<&str> = differing.split('\0').filter(|s| !s.is_empty()).collect();
    let changed: Vec<String> = steps.iter().map(Step::path).filter(|p| differing.contains(p)).map(String::from).collect();
    if !changed.is_empty() {
        return Err(UndoError::ChangedSince(changed));
    }

    // 2. Read every old content now, so a failure here leaves the tree untouched.
    let wanted: Vec<&str> = steps.iter().filter_map(|s| matches!(s, Step::Restore { .. }).then(|| s.path())).collect();
    let mut entries = HashMap::new();
    for chunk in wanted.chunks(200) {
        let mut args = vec!["--literal-pathspecs", "ls-tree", "-z", summary.before.as_str(), "--"];
        args.extend(chunk);
        let listing = git_text(&root, None, &args).ok_or_else(|| UndoError::Failed("could not read the old tree".into()))?;
        entries.extend(parse_ls_tree(&listing));
    }
    let mut blobs: HashMap<&str, Blob> = HashMap::new();
    for path in &wanted {
        let (kind, hash) = entries
            .get(*path)
            .ok_or_else(|| UndoError::Failed(format!("{path} is not in the tree from before the turn (or is a submodule)")))?;
        let bytes = git(&root, None, &["cat-file", "blob", hash]).ok_or_else(|| UndoError::Failed(format!("could not read the old {path}")))?;
        blobs.insert(path, Blob { kind: *kind, bytes });
    }

    // 3. Every target must really be inside the repository.
    for step in &steps {
        if !inside_root(&root_canon, &root.join(step.path())) {
            return Err(UndoError::UnsafePath(step.path().to_string()));
        }
    }

    // 4. Apply. Deletions first, so a rename back never meets its own leftover.
    let mut report = UndoReport::default();
    for step in &steps {
        if let Step::Delete(path) = step {
            remove_file_and_empty_parents(&root, path)?;
            report.removed.push(path.clone());
        }
    }
    for step in &steps {
        if let Step::Restore { path, recreated } = step {
            write_blob(&root, path, &blobs[path.as_str()])?;
            if *recreated { &mut report.recreated } else { &mut report.restored }.push(path.clone());
        }
    }
    Ok(report)
}

fn remove_file_and_empty_parents(root: &Path, rel: &str) -> Result<(), UndoError> {
    let target = root.join(rel);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => std::fs::remove_file(&target).map_err(|e| io_err("could not delete", rel, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("could not look at", rel, e)),
    }
    // Directories the turn created for this file go with it; a non-empty one stays.
    let mut dir = target.parent();
    while let Some(d) = dir.filter(|d| *d != root) {
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

/// Write through a temp file and rename, so a symlink (or a read-only file) in the way
/// is replaced rather than followed.
fn write_blob(root: &Path, rel: &str, blob: &Blob) -> Result<(), UndoError> {
    let target = root.join(rel);
    let parent = target.parent().unwrap_or(root);
    std::fs::create_dir_all(parent).map_err(|e| io_err("could not create the folder of", rel, e))?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.jev-undo"));
    let _ = std::fs::remove_file(&tmp);
    let written = match blob.kind {
        #[cfg(unix)]
        Kind::Symlink => {
            use std::os::unix::ffi::OsStrExt;
            std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&blob.bytes), &tmp)
        }
        #[cfg(not(unix))]
        Kind::Symlink => std::fs::write(&tmp, &blob.bytes),
        Kind::File { executable } => std::fs::write(&tmp, &blob.bytes).and_then(|()| set_executable(&tmp, executable)),
    };
    written.map_err(|e| io_err("could not write", rel, e))?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err("could not replace", rel, e)
    })
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_executable(_: &Path, _: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!("jev-changes-{}-{name}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(std::fs::canonicalize(p).unwrap())
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.0.join(rel)).unwrap()
        }
        fn exists(&self, rel: &str) -> bool {
            std::fs::symlink_metadata(self.0.join(rel)).is_ok()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A repo with a.txt (3 lines), b.txt, c.txt committed and `*.log` ignored.
    fn repo() -> Scratch {
        let s = Scratch::new("repo");
        run_git(s.path(), &["init", "-q"]);
        s.write(".gitignore", "*.log\n");
        s.write("a.txt", "one\ntwo\nthree\n");
        s.write("b.txt", "bee\n");
        s.write("src/c.txt", "see\n");
        run_git(s.path(), &["add", "-A"]);
        run_git(s.path(), &["commit", "-q", "-m", "init"]);
        s
    }

    /// What a turn did: modify a.txt, add new.txt, delete b.txt, and write an ignored file.
    fn fake_turn(s: &Scratch) {
        s.write("a.txt", "one\nTWO\nthree\nfour\n");
        s.write("new.txt", "x\ny\nz\n");
        std::fs::remove_file(s.path().join("b.txt")).unwrap();
        s.write("debug.log", "noise\n");
    }

    fn files_by_path(sum: &ChangeSummary) -> HashMap<&str, &FileChange> {
        sum.files.iter().map(|f| (f.path.as_str(), f)).collect()
    }

    #[test]
    fn name_status_and_numstat_parse_including_renames_and_binary() {
        let ns = "M\0a.txt\0A\0new.txt\0D\0gone.txt\0R087\0old name.txt\0new name.txt\0T\0t.bin\0";
        let num = "1\t2\ta.txt\03\t0\tnew.txt\00\t0\t\0old name.txt\0new name.txt\0-\t-\tt.bin\0";
        let files = build_files(ns, num);
        let by: HashMap<_, _> = files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(files.len(), 5);
        assert_eq!((by["a.txt"].status, by["a.txt"].added, by["a.txt"].removed), (ChangeStatus::Modified, 1, 2));
        assert_eq!((by["new.txt"].status, by["new.txt"].added), (ChangeStatus::Added, 3));
        assert_eq!(by["gone.txt"].status, ChangeStatus::Deleted);
        assert_eq!(by["new name.txt"].status, ChangeStatus::Renamed);
        assert_eq!(by["new name.txt"].old_path.as_deref(), Some("old name.txt"));
        assert_eq!((by["t.bin"].status, by["t.bin"].added, by["t.bin"].removed), (ChangeStatus::Modified, 0, 0));
        assert!(parse_name_status("").is_empty() && parse_numstat("").is_empty());
        assert!(parse_name_status("R100\0only-one\0").is_empty(), "truncated output is dropped, not guessed");
    }

    #[test]
    fn only_safe_relative_paths_are_accepted() {
        for ok in ["a.txt", "src/c.txt", "dir/.hidden", "a b/c d.rs", ".gitignore", ".github/x.yml"] {
            assert!(is_safe_rel_path(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a/../b", ".git/config", "x/.git/hooks/pre-commit", ".GIT/config", "./a", "a\u{FFFD}b"] {
            assert!(!is_safe_rel_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_undo_plan_covers_each_kind_and_refuses_unsafe_paths_up_front() {
        let fc = |path: &str, status, old: Option<&str>| FileChange { path: path.into(), status, added: 0, removed: 0, old_path: old.map(String::from) };
        let plan = plan_undo(&[
            fc("m", ChangeStatus::Modified, None),
            fc("a", ChangeStatus::Added, None),
            fc("d", ChangeStatus::Deleted, None),
            fc("new", ChangeStatus::Renamed, Some("old")),
        ])
        .unwrap();
        assert_eq!(
            plan,
            vec![
                Step::Restore { path: "m".into(), recreated: false },
                Step::Delete("a".into()),
                Step::Restore { path: "d".into(), recreated: true },
                Step::Delete("new".into()),
                Step::Restore { path: "old".into(), recreated: true },
            ]
        );
        assert_eq!(plan_undo(&[]), Err(UndoError::NothingToUndo));
        let err = plan_undo(&[fc("ok", ChangeStatus::Added, None), fc("../../etc/passwd", ChangeStatus::Modified, None)]);
        assert_eq!(err, Err(UndoError::UnsafePath("../../etc/passwd".into())));
        let err = plan_undo(&[fc("new", ChangeStatus::Renamed, Some("/abs"))]);
        assert_eq!(err, Err(UndoError::UnsafePath("/abs".into())));
    }

    #[test]
    fn ls_tree_parses_modes_and_skips_submodules() {
        let raw = "100644 blob aaa\ta.txt\0100755 blob bbb\trun.sh\0120000 blob ccc\tlink\0160000 commit ddd\tsub\0";
        let m = parse_ls_tree(raw);
        assert_eq!(m["a.txt"].0, Kind::File { executable: false });
        assert_eq!(m["run.sh"].0, Kind::File { executable: true });
        assert_eq!(m["link"].0, Kind::Symlink);
        assert!(!m.contains_key("sub"));
    }

    #[test]
    fn a_snapshot_sees_untracked_files_skips_ignored_ones_and_touches_nothing() {
        let s = repo();
        let status_before = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
        let index_before = std::fs::read(s.path().join(".git/index")).unwrap();
        let t1 = snapshot(s.path()).expect("a tree id");
        s.write("untracked.txt", "u\n");
        s.write("ignored.log", "i\n");
        let t2 = snapshot(s.path()).unwrap();
        assert_ne!(t1, t2);
        let listing = run_git(s.path(), &["ls-tree", "-r", "--name-only", &t2]);
        assert!(listing.contains("untracked.txt") && !listing.contains("ignored.log"), "{listing}");
        // The user's index is as it was and the new file is still untracked, not staged.
        assert_eq!(std::fs::read(s.path().join(".git/index")).unwrap(), index_before);
        let status = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
        assert!(status.contains("?? untracked.txt"), "{status}");
        assert_ne!(status, status_before);
        // Same content, same tree.
        assert_eq!(snapshot(s.path()).unwrap(), t2);
    }

    #[test]
    fn the_summary_lists_added_modified_and_deleted_files_with_line_counts() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        fake_turn(&s);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!((sum.before.as_str(), sum.after.as_str()), (before.as_str(), after.as_str()));
        let by = files_by_path(&sum);
        assert_eq!(sum.files.len(), 3, "{:?}", sum.files);
        assert_eq!((by["a.txt"].status, by["a.txt"].added, by["a.txt"].removed), (ChangeStatus::Modified, 2, 1));
        assert_eq!((by["new.txt"].status, by["new.txt"].added, by["new.txt"].removed), (ChangeStatus::Added, 3, 0));
        assert_eq!((by["b.txt"].status, by["b.txt"].added, by["b.txt"].removed), (ChangeStatus::Deleted, 0, 1));
        assert!(!by.contains_key("debug.log"), "ignored files are not changes");
        assert_eq!(sum.totals(), (5, 2));

        let diff = diff_text(s.path(), &before, &after, None).unwrap();
        assert!(diff.contains("+TWO") && diff.contains("-two") && diff.contains("+++ b/new.txt") && diff.contains("--- a/b.txt"), "{diff}");
        assert!(!diff.contains('\u{1b}'), "no colour codes");
        let one = diff_text(s.path(), &before, &after, Some("new.txt")).unwrap();
        assert!(one.contains("new.txt") && !one.contains("a.txt"), "{one}");
        // A glob is a name here, not a pattern.
        assert_eq!(diff_text(s.path(), &before, &after, Some("*.txt")).unwrap(), "");
    }

    #[test]
    fn nothing_changed_gives_an_empty_summary() {
        let s = repo();
        let t = snapshot(s.path()).unwrap();
        assert!(summarize(s.path(), &t, &snapshot(s.path()).unwrap()).unwrap().is_empty());
    }

    #[test]
    fn a_moved_file_is_a_rename_and_undo_moves_it_back() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        std::fs::rename(s.path().join("a.txt"), s.path().join("renamed.txt")).unwrap();
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(sum.files.len(), 1, "{:?}", sum.files);
        let f = &sum.files[0];
        assert_eq!((f.status, f.path.as_str(), f.old_path.as_deref()), (ChangeStatus::Renamed, "renamed.txt", Some("a.txt")));
        let report = undo(s.path(), &sum).unwrap();
        assert_eq!(report.removed, ["renamed.txt"]);
        assert_eq!(report.recreated, ["a.txt"]);
        assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
        assert!(!s.exists("renamed.txt"));
    }

    #[test]
    fn undo_restores_modified_added_and_deleted_files_and_leaves_ignored_ones_alone() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("deep/er/new.txt", "x\n");
        fake_turn(&s);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();

        let report = undo(s.path(), &sum).unwrap();
        assert_eq!(report.restored, ["a.txt"]);
        assert_eq!(report.recreated, ["b.txt"]);
        let mut removed = report.removed.clone();
        removed.sort();
        assert_eq!(removed, ["deep/er/new.txt", "new.txt"]);
        assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
        assert_eq!(s.read("b.txt"), "bee\n");
        assert!(!s.exists("new.txt") && !s.exists("deep"), "the folders the turn made go too");
        assert_eq!(s.read("debug.log"), "noise\n", "an ignored file is not ours to touch");
        assert_eq!(snapshot(s.path()).unwrap(), before, "the tree is exactly as before the turn");
        // The repository's own state was never touched.
        assert!(run_git(s.path(), &["status", "--porcelain"]).trim().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn undo_puts_the_executable_bit_and_symlinks_back() {
        use std::os::unix::fs::PermissionsExt;
        let s = repo();
        s.write("run.sh", "#!/bin/sh\n");
        std::fs::set_permissions(s.path().join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("a.txt", s.path().join("link")).unwrap();
        run_git(s.path(), &["add", "-A"]);
        run_git(s.path(), &["commit", "-q", "-m", "more"]);
        let before = snapshot(s.path()).unwrap();
        std::fs::remove_file(s.path().join("run.sh")).unwrap();
        std::fs::remove_file(s.path().join("link")).unwrap();
        s.write("link", "now a plain file\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        undo(s.path(), &sum).unwrap();
        let mode = std::fs::metadata(s.path().join("run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
        assert_eq!(std::fs::read_link(s.path().join("link")).unwrap(), Path::new("a.txt"));
        assert_eq!(snapshot(s.path()).unwrap(), before);
    }

    #[test]
    fn undo_refuses_when_a_changed_file_was_edited_again_and_touches_nothing() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        fake_turn(&s);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();

        s.write("new.txt", "x\ny\nz\nand the user added this\n");
        s.write("unrelated.txt", "not part of the turn\n");
        let err = undo(s.path(), &sum).unwrap_err();
        assert_eq!(err, UndoError::ChangedSince(vec!["new.txt".into()]), "only the edited file is named, not unrelated ones");
        assert!(err.to_string().contains("new.txt"));
        assert_eq!(s.read("a.txt"), "one\nTWO\nthree\nfour\n", "nothing was undone");
        assert!(!s.exists("b.txt"));

        // A deleted file that came back, and a modified one edited again, are caught too.
        s.write("new.txt", "x\ny\nz\n");
        s.write("b.txt", "back again\n");
        s.write("a.txt", "edited again\n");
        match undo(s.path(), &sum).unwrap_err() {
            UndoError::ChangedSince(mut files) => {
                files.sort();
                assert_eq!(files, ["a.txt", "b.txt"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn undo_works_from_a_subdirectory_of_the_repository() {
        let s = repo();
        let sub = s.path().join("src");
        let before = snapshot(&sub).unwrap();
        fake_turn(&s);
        let after = snapshot(&sub).unwrap();
        let sum = summarize(&sub, &before, &after).unwrap();
        assert!(files_by_path(&sum).contains_key("a.txt"), "paths are relative to the repository root");
        undo(&sub, &sum).unwrap();
        assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
    }

    #[test]
    fn undo_refuses_a_summary_with_a_path_outside_the_repository() {
        let s = repo();
        let t = snapshot(s.path()).unwrap();
        let evil = ChangeSummary {
            files: vec![FileChange { path: "../escape.txt".into(), status: ChangeStatus::Added, added: 1, removed: 0, old_path: None }],
            before: t.clone(),
            after: t,
        };
        std::fs::write(s.path().parent().unwrap().join("escape.txt"), "mine").ok();
        assert!(matches!(undo(s.path(), &evil), Err(UndoError::UnsafePath(_))));
        assert_eq!(std::fs::read_to_string(s.path().parent().unwrap().join("escape.txt")).unwrap(), "mine");
        let _ = std::fs::remove_file(s.path().parent().unwrap().join("escape.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_links_out_of_the_repository_is_not_followed() {
        let s = repo();
        let outside = Scratch::new("outside");
        std::os::unix::fs::symlink(outside.path(), s.path().join("sneaky")).unwrap();
        let root = std::fs::canonicalize(s.path()).unwrap();
        assert!(!inside_root(&root, &s.path().join("sneaky/file.txt")));
        assert!(!inside_root(&root, &s.path().join("sneaky/new/dir/file.txt")), "not-yet-existing parents are resolved from the nearest real one");
        assert!(inside_root(&root, &s.path().join("src/new/file.txt")));
    }

    #[test]
    fn outside_a_repository_everything_degrades_to_none() {
        let s = Scratch::new("plain");
        s.write("f.txt", "x");
        assert_eq!(snapshot(s.path()), None);
        let t = "0".repeat(40);
        assert_eq!(summarize(s.path(), &t, &"1".repeat(40)), None);
        assert_eq!(diff_text(s.path(), &t, &t, None), None);
        let sum = ChangeSummary {
            files: vec![FileChange { path: "f.txt".into(), status: ChangeStatus::Added, added: 1, removed: 0, old_path: None }],
            before: t.clone(),
            after: t,
        };
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::Failed(_))));
        assert_eq!(snapshot(Path::new("/definitely/not/a/dir")), None);
    }

    #[test]
    fn a_fresh_repository_without_commits_can_be_snapshotted() {
        let s = Scratch::new("fresh");
        run_git(s.path(), &["init", "-q"]);
        let before = snapshot(s.path()).expect("empty tree");
        s.write("first.txt", "hi\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(sum.files.len(), 1);
        undo(s.path(), &sum).unwrap();
        assert!(!s.exists("first.txt"));
    }
}
