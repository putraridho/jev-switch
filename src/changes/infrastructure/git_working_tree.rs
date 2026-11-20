//! What one turn changed in the working tree: a summary, the diff, and an undo.
//!
//! A [`Snapshot`] has two parts, taken before and after the turn:
//!
//! * the **tracked** content, as a git *tree object* built through a temporary index
//!   (a copy of the real one, plus `git add -u`). Only files the user already gave to git
//!   are ever hashed into `.git/objects`; summary, diff and undo are complete for them.
//! * a cheap **manifest** of the untracked, non-ignored files (path, size, mtime; no
//!   content is read). A file that is untracked now and was not in the manifest was added
//!   by the turn, so undo may delete it. One that was there and changed (or vanished) is
//!   [`ChangeStatus::ModifiedUntracked`]: we never had its old content, so undo refuses.
//!
//! Ignored files are never looked at, so an undo cannot reach them. The user's index,
//! branch and working tree are never touched by a snapshot. Everything here is best
//! effort: no git, not a repository or any git failure gives `None`, and a turn never
//! fails because of it.
//!
//! Layout: the parsers, the manifest merge, the undo plan and the "edited since" rule are
//! pure and tested on values; the few functions that run `git` or touch files are thin and
//! tested in real temp repos.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// An untracked file that existed before the turn and was changed or removed by it.
    /// There is no earlier copy, so it is reported but never undone.
    ModifiedUntracked,
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

impl FileChange {
    fn new(path: String, status: ChangeStatus) -> Self {
        Self { path, status, added: 0, removed: 0, old_path: None }
    }
}

/// What a file looked like from the outside: enough to tell it changed, without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ns: i128,
}

/// The working tree at one moment; see the module docs. Two snapshots are equal when
/// the working tree is the same (tree and untracked files), whenever they were taken.
#[derive(Debug, Clone, Eq)]
pub struct Snapshot {
    /// Tree id of the tracked files, as they are in the working tree.
    pub tree: String,
    /// Untracked, non-ignored files by repository-relative path.
    pub untracked: HashMap<String, Stamp>,
    /// Wall-clock time the snapshot began (ns since the epoch; 0 = unknown). A file
    /// that shows up untracked after the turn but is OLDER than this existed before,
    /// hidden by `.gitignore` (the turn un-ignored it): it is never treated as created
    /// by the turn, so undo can never delete it.
    pub taken_ns: i128,
    /// HEAD commit and branch when the snapshot was taken: if either differs between the
    /// two snapshots of a turn, somebody else moved the repository during it.
    pub repo: RepoState,
}

/// Where the repository's HEAD pointed: the commit (`None` before the first commit) and
/// the branch (`None` when detached).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoState {
    pub head: Option<String>,
    pub branch: Option<String>,
}

impl RepoState {
    fn label(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(b), _) => b.clone(),
            (None, Some(h)) => format!("detached at {}", &h[..h.len().min(8)]),
            (None, None) => "no commits".to_string(),
        }
    }
}

/// HEAD (or its branch) moved while the turn ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMove {
    pub from: RepoState,
    pub to: RepoState,
}

impl RepoMove {
    /// `branch switched dashboard → master`, `branch dashboard moved to another commit`.
    pub fn describe(&self) -> String {
        if self.from.branch != self.to.branch {
            format!("branch switched {} → {}", self.from.label(), self.to.label())
        } else if self.from.branch.is_some() {
            format!("branch {} moved to another commit", self.from.label())
        } else {
            format!("HEAD moved: {} → {}", self.from.label(), self.to.label())
        }
    }
}

/// Changes the working tree went through during the turn that the turn did not make
/// (as far as can be told). Shown as a warning, never undone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutsideChanges {
    pub files: Vec<FileChange>,
    /// Set when HEAD or the branch moved during the turn.
    pub moved: Option<RepoMove>,
    /// A shell command (or a write to an unknown path) ran, so files in `ChangeSummary::files`
    /// that the turn's own edit tools did not name may still be someone else's.
    pub uncertain: bool,
}

impl OutsideChanges {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.moved.is_none() && !self.uncertain
    }

    /// The one warning line for the end of a turn; `None` when there is nothing to warn about.
    pub fn warning(&self) -> Option<String> {
        let n = self.files.len();
        let files = if n == 1 { "1 file".to_string() } else { format!("{n} files") };
        match (&self.moved, n) {
            (Some(m), 1..) => Some(format!("{files} changed outside this turn ({}); not included, /undo won't touch them", m.describe())),
            (Some(m), 0) => Some(format!("{}; /undo is disabled for this turn", m.describe())),
            (None, 1..) => Some(format!("{files} changed outside this turn (another process); not included, /undo won't touch them")),
            (None, 0) if self.uncertain => Some("a shell command ran, so the files listed may include changes from other processes".to_string()),
            (None, 0) => None,
        }
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.tree == other.tree && self.untracked == other.untracked
    }
}

/// How much older than the snapshot a "new" file may look before it counts as having
/// existed already (filesystem timestamp granularity and clock skew).
const PRE_EXISTING_SLACK_NS: i128 = 2_000_000_000;

/// The difference between two working-tree snapshots. `files` is never empty in a
/// summary the engine keeps: a turn that changed nothing has no summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSummary {
    pub files: Vec<FileChange>,
    /// Tree ids of the tracked files before and after the turn.
    pub before: String,
    pub after: String,
    /// The untracked files the turn added, as they were when it ended: undo deletes one
    /// only while it still looks like this.
    pub added_stamps: HashMap<String, Stamp>,
    /// HEAD and branch when the turn ended; `/undo` refuses if they are different later.
    pub repo_after: RepoState,
    /// What changed that this turn did not do; see [`OutsideChanges`].
    pub outside: OutsideChanges,
}

impl ChangeSummary {
    /// Nothing to show: no attributable file and nothing to warn about.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.outside.is_empty()
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

/// `git ls-files --others -z`: the untracked paths. A trailing `/` is a nested repository
/// (not ours to track); a path mangled by lossy decoding cannot be addressed again.
fn parse_untracked_list(raw: &str) -> Vec<&str> {
    raw.split('\0').filter(|p| !p.is_empty() && !p.ends_with('/') && !p.contains('\u{FFFD}')).collect()
}

// ------------------------------------------------------------- tracked + untracked (pure)

/// The tracked diff of two snapshots combined with what their manifests say.
struct Merged {
    files: Vec<FileChange>,
    /// The untracked files the turn added, with their stamps now.
    added: HashMap<String, Stamp>,
}

/// The untracked paths an addition pass splits into added and revealed, with their stamps.
type StampedPaths<'a> = (Vec<(&'a String, &'a Stamp)>, Vec<(&'a String, &'a Stamp)>);

/// Fold the untracked manifests into the tracked file list.
///
/// * untracked now, not before: `Added` (unless it is a tracked file that left the index,
///   which the tree diff already reports as deleted, and which is still there);
/// * untracked before and changed or gone now: `ModifiedUntracked`;
/// * a pre-existing untracked file the turn only staged shows as added in the tree diff:
///   that is dropped (and caught above if its content changed), so an undo never deletes
///   a file that was already there.
///
/// `stamp_now` looks a path up on disk, for files that are no longer in the manifest.
fn merge_untracked(tree_files: Vec<FileChange>, before: &Snapshot, after: &Snapshot, stamp_now: impl Fn(&str) -> Option<Stamp>) -> Merged {
    let deleted: HashSet<String> = tree_files.iter().filter(|f| f.status == ChangeStatus::Deleted).map(|f| f.path.clone()).collect();
    let mut files = Vec::new();
    for f in tree_files {
        let existed_untracked = before.untracked.contains_key(&f.path);
        match f.status {
            ChangeStatus::Deleted if after.untracked.contains_key(&f.path) => {}
            ChangeStatus::Added if existed_untracked => {}
            ChangeStatus::Renamed if existed_untracked => {
                if let Some(old) = f.old_path {
                    files.push(FileChange::new(old, ChangeStatus::Deleted));
                }
            }
            _ => files.push(f),
        }
    }

    let mut modified: Vec<&String> = before
        .untracked
        .iter()
        .filter(|(p, was)| after.untracked.get(*p).copied().or_else(|| stamp_now(p)) != Some(**was))
        .map(|(p, _)| p)
        .collect();
    modified.sort();
    files.extend(modified.into_iter().map(|p| FileChange::new(p.clone(), ChangeStatus::ModifiedUntracked)));

    let (mut added, revealed): StampedPaths<'_> = after
        .untracked
        .iter()
        .filter(|(p, _)| !before.untracked.contains_key(*p) && !deleted.contains(*p))
        .partition(|(_, st)| before.taken_ns == 0 || st.mtime_ns >= before.taken_ns - PRE_EXISTING_SLACK_NS);
    // Older than the turn: it was there all along, hidden by .gitignore. Report it like
    // a changed untracked file, so undo refuses rather than deleting the user's file.
    let mut revealed: Vec<&String> = revealed.into_iter().map(|(p, _)| p).collect();
    revealed.sort();
    files.extend(revealed.into_iter().map(|p| FileChange::new(p.clone(), ChangeStatus::ModifiedUntracked)));
    added.sort_by(|a, b| a.0.cmp(b.0));
    files.extend(added.iter().map(|(p, _)| FileChange::new((*p).clone(), ChangeStatus::Added)));
    Merged { files, added: added.into_iter().map(|(p, s)| (p.clone(), *s)).collect() }
}

// ---------------------------------------------------------------------- git (thin IO)

/// Run `git -C dir args...` (optionally on another index) and return stdout.
fn git(dir: &Path, index: Option<&Path>, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(crate::shared::infrastructure::git_guard::no_repo_programs(dir)).args(args).stdin(Stdio::null()).stderr(Stdio::null());
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
    let nanos = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("jev-index-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// Copy the real index to `tmp` AND give the copy the original's modification time.
/// Git's "racily clean" protection re-reads any file whose mtime is not older than the
/// index file's own mtime; a plain copy gets a fresh mtime, which disables that
/// protection, so a same-size edit made within the same second as the last index write
/// (e.g. "two" -> "TWO") would be trusted from the stat cache and missed by the
/// snapshot. Keeping the original mtime keeps git's check intact.
///
/// True when `tmp` is a trustworthy copy. On any failure `tmp` is removed (a failed copy
/// can leave a truncated file that git would read as the index) and the caller must seed
/// the index another way. The copy itself is injected so tests can fail it.
fn copy_index_with(real: &Path, tmp: &Path, copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>) -> bool {
    let ok = copy(real, tmp).is_ok() && {
        let mtime = std::fs::metadata(real).and_then(|m| m.modified());
        let file = std::fs::OpenOptions::new().write(true).open(tmp);
        // Without the original mtime the stat cache cannot be trusted.
        matches!((mtime, file), (Ok(mtime), Ok(file)) if file.set_modified(mtime).is_ok())
    };
    if !ok {
        let _ = std::fs::remove_file(tmp);
    }
    ok
}

/// Build `tmp` from the real index's entries (mode, object, path) when it cannot be
/// copied. It carries no stat cache, so git hashes every tracked file: slower, correct.
/// An absent index is NOT the same: git would see no tracked files and the snapshot would
/// be the empty tree, so a failure here means no snapshot at all.
///
/// `--index-info` takes no flags, so the skip-worktree bit (a sparse checkout, or
/// `update-index --skip-worktree`) is put back from `ls-files -t` (tag `S`): without it
/// `add -u` would record every file that is not checked out as deleted. If that cannot be
/// done there is no snapshot ("changes not tracked this turn"), never a wrong tree. An
/// intent-to-add entry needs nothing: `add -u` stages its content either way.
fn seed_index_from_entries(root: &Path, tmp: &Path) -> Option<()> {
    let entries = git(root, None, &["ls-files", "--stage", "-z"])?;
    git_with_input(root, tmp, &["update-index", "-z", "--index-info"], &entries)?;
    let tagged = git(root, None, &["ls-files", "-t", "-z"])?;
    let skipped = skip_worktree_paths(&tagged);
    if skipped.is_empty() {
        return Some(());
    }
    git_with_input(root, tmp, &["update-index", "-z", "--skip-worktree", "--stdin"], &skipped)
}

/// The NUL-separated paths `ls-files -t -z` tags `S` (skip-worktree), each NUL-terminated.
fn skip_worktree_paths(tagged: &[u8]) -> Vec<u8> {
    let mut paths = Vec::new();
    for path in tagged.split(|b| *b == 0).filter_map(|entry| entry.strip_prefix(b"S ")) {
        paths.extend_from_slice(path);
        paths.push(0);
    }
    paths
}

/// Run `git -C root args...` on the index `index`, with `input` on stdin; `Some` when it succeeded.
fn git_with_input(root: &Path, index: &Path, args: &[&str], input: &[u8]) -> Option<()> {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(crate::shared::infrastructure::git_guard::no_repo_programs(root))
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_INDEX_FILE", index)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let written = child.stdin.take().map(|mut stdin| stdin.write_all(input).is_ok());
    let succeeded = child.wait().ok()?.success();
    (written == Some(true) && succeeded).then_some(())
}

fn remove_temp_index(tmp: PathBuf) {
    let _ = std::fs::remove_file(&tmp);
    let mut lock = tmp.into_os_string();
    lock.push(".lock");
    let _ = std::fs::remove_file(lock);
}

/// The repository root and where its own index is, in one git run.
fn repo_paths(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let out = git_text(dir, None, &["rev-parse", "--show-toplevel", "--git-path", "index"])?;
    let (root, index) = out.split_once('\n')?;
    let (root, index) = (root.trim_end_matches('\r'), index.trim_end_matches(['\n', '\r']));
    if root.is_empty() || index.is_empty() {
        return None;
    }
    let index = PathBuf::from(index);
    Some((PathBuf::from(root), if index.is_absolute() { index } else { dir.join(index) }))
}

fn toplevel(dir: &Path) -> Option<PathBuf> {
    repo_paths(dir).map(|(root, _)| root)
}

/// The repository root of `dir`, for callers that apply patches against it.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    toplevel(dir)
}

fn stamp_of(root: &Path, rel: &str) -> Option<Stamp> {
    let meta = std::fs::symlink_metadata(root.join(rel)).ok()?;
    let mtime_ns = match meta.modified().ok()?.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    Some(Stamp { size: meta.len(), mtime_ns })
}

/// The untracked, non-ignored files and their stamps. Reads no file content.
fn untracked_manifest(root: &Path) -> Option<HashMap<String, Stamp>> {
    let listing = git_text(root, None, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    Some(parse_untracked_list(&listing).into_iter().filter_map(|p| Some((p.to_string(), stamp_of(root, p)?))).collect())
}

/// The state of the working tree in `dir` right now; `None` outside a repository or if
/// git fails. Hashes only files git already tracks (into a temporary index, never the
/// real one), and only stats the untracked ones. Blocking: call it from `spawn_blocking`,
/// or use [`snapshot_until`].
pub fn snapshot(dir: &Path) -> Option<Snapshot> {
    snapshot_with(dir, |from, to| std::fs::copy(from, to))
}

/// [`snapshot`] with the index copy injected.
fn snapshot_with(dir: &Path, copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>) -> Option<Snapshot> {
    let taken_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);
    let (root, real) = repo_paths(dir)?;
    let tmp = temp_index_path();
    // Starting from a copy of the real index keeps git's stat cache, so only files that
    // really changed are hashed again; without one it still works, just slower.
    let seeded = !real.is_file() || copy_index_with(&real, &tmp, copy) || seed_index_from_entries(&root, &tmp).is_some();
    let tree = seeded
        .then(|| git(&root, Some(&tmp), &["add", "-u"]))
        .flatten()
        .and_then(|_| git_text(&root, Some(&tmp), &["write-tree"]))
        .map(|t| t.trim().to_string())
        .filter(|t| is_object_id(t));
    remove_temp_index(tmp);
    Some(Snapshot { tree: tree?, untracked: untracked_manifest(&root)?, taken_ns, repo: repo_state(&root) })
}

/// HEAD commit and branch now. Missing pieces (no commit yet, detached) are `None`.
fn repo_state(root: &Path) -> RepoState {
    let one = |args: &[&str]| git_text(root, None, args).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    RepoState { head: one(&["rev-parse", "-q", "--verify", "HEAD"]), branch: one(&["symbolic-ref", "-q", "--short", "HEAD"]) }
}

/// What changed between two snapshots. `Some` with no files when nothing did.
pub fn summarize(dir: &Path, before: &Snapshot, after: &Snapshot) -> Option<ChangeSummary> {
    let root = toplevel(dir)?;
    let tree_files = if before.tree == after.tree {
        Vec::new()
    } else {
        let diff = |mode: &str| git_text(&root, None, &["diff", mode, "-z", "-M", "--no-ext-diff", "--no-textconv", &before.tree, &after.tree]);
        build_files(&diff("--name-status")?, &diff("--numstat")?)
    };
    let Merged { mut files, added } = merge_untracked(tree_files, before, after, |p| stamp_of(&root, p));
    for f in files.iter_mut().filter(|f| f.status == ChangeStatus::Added && added.contains_key(&f.path)) {
        f.added = new_file_lines(&root, &f.path);
    }
    Some(ChangeSummary {
        files,
        before: before.tree.clone(),
        after: after.tree.clone(),
        added_stamps: added,
        repo_after: after.repo.clone(),
        outside: OutsideChanges::default(),
    })
}

// ------------------------------------------------------- what the turn itself did (pure)

/// What a turn's own tools did, from the tool lines the backends report: the paths its
/// edit tools wrote, and whether anything ran that could write anywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnActivity {
    /// Paths named by edit/write tools, as reported (absolute or relative to the turn's folder).
    pub written: Vec<String>,
    /// A shell command, a sub-agent, an unknown tool, or a write tool without a usable
    /// path ran: it may have changed any file.
    pub opaque: bool,
    /// At least one tool line was reported. A turn that reported none may still have changed
    /// files (some CLIs report no tool events), so nothing can be told apart then.
    pub reported: bool,
}

/// Tools that write the one file their line names.
const WRITE_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "edit", "write_file", "replace", "edit_file", "write", "apply_patch", "create_file"];
/// Tools known not to change files.
const READ_TOOLS: &[&str] = &[
    "Read", "Grep", "Glob", "LS", "WebFetch", "WebSearch", "TodoWrite", "ExitPlanMode", "read_file", "list_directory", "glob", "grep",
    "search_file_content", "read_many_files", "web_fetch", "google_web_search", "web_search",
];
/// `tool_summary` cuts the telling field at this many characters; a path that long is cut.
const TOOL_FIELD_LIMIT: usize = 100;

impl TurnActivity {
    /// Classify tool lines such as `Edit /p/a.rs`, `edit /p/b.rs`, `Bash cargo test`, `Grep foo`.
    pub fn from_tools(tools: &[String]) -> Self {
        let mut a = Self::default();
        a.add_tools(tools);
        a
    }

    pub fn add_tools(&mut self, tools: &[String]) {
        for line in tools {
            self.reported = true;
            let (name, rest) = line.split_once(' ').map_or((line.as_str(), ""), |(n, r)| (n, r.trim()));
            if READ_TOOLS.contains(&name) {
                continue;
            }
            if WRITE_TOOLS.contains(&name) && !rest.is_empty() && rest.chars().count() < TOOL_FIELD_LIMIT {
                self.written.push(rest.to_string());
            } else {
                self.opaque = true;
            }
        }
    }
}

fn normalize_lexically(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A written path as repository-relative text, or `None` when it is outside the repository.
fn relative_to_root(root: &Path, dir: &Path, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    let abs = normalize_lexically(&if path.is_absolute() { path.to_path_buf() } else { dir.join(path) });
    let canon_root = std::fs::canonicalize(root).ok();
    // The folder may be reached through a link: resolve the parent, keep the file name.
    let via_parent = abs.parent().and_then(|p| std::fs::canonicalize(p).ok()).zip(abs.file_name()).map(|(p, n)| p.join(n));
    [Some(abs), via_parent].into_iter().flatten().find_map(|candidate| {
        let bases = std::iter::once(root).chain(canon_root.as_deref());
        bases.into_iter().find_map(|b| candidate.strip_prefix(b).ok().map(|r| r.to_string_lossy().into_owned()))
    })
}

/// Split `files` (everything that differs between the two snapshots) into what this turn
/// did and what somebody else did, given what the turn's tools reported.
///
/// * HEAD or the branch moved: the diff is untrustworthy. Only files the edit tools named
///   stay (and none at all if something opaque ran); `/undo` is refused separately.
/// * a shell command ran: every file stays, marked uncertain unless the tools named them all;
/// * otherwise only the files the edit tools named stay.
fn attribute(files: Vec<FileChange>, written: &HashSet<String>, opaque: bool, moved: Option<RepoMove>) -> (Vec<FileChange>, OutsideChanges) {
    let named = |f: &FileChange| written.contains(&f.path) || f.old_path.as_ref().is_some_and(|o| written.contains(o));
    if opaque && moved.is_none() {
        let uncertain = files.iter().any(|f| !named(f));
        return (files, OutsideChanges { files: Vec::new(), moved: None, uncertain });
    }
    let (mine, outside): (Vec<FileChange>, Vec<FileChange>) = if opaque { (Vec::new(), files) } else { files.into_iter().partition(named) };
    (mine, OutsideChanges { files: outside, moved, uncertain: false })
}

/// [`summarize`], then drop what other processes changed during the turn (see [`attribute`]).
pub fn summarize_turn(dir: &Path, before: &Snapshot, after: &Snapshot, activity: &TurnActivity) -> Option<ChangeSummary> {
    let mut summary = summarize(dir, before, after)?;
    let root = toplevel(dir)?;
    // A write outside the repository cannot be among the changed files: it is dropped.
    let written: HashSet<String> = activity.written.iter().filter_map(|w| relative_to_root(&root, dir, w)).collect();
    let moved = (before.repo != after.repo).then(|| RepoMove { from: before.repo.clone(), to: after.repo.clone() });
    // No tool lines at all: the turn's edits (if any) went unreported, so treat it as opaque.
    let opaque = activity.opaque || !activity.reported;
    let (files, outside) = attribute(std::mem::take(&mut summary.files), &written, opaque, moved);
    summary.added_stamps.retain(|p, _| files.iter().any(|f| &f.path == p));
    summary.files = files;
    summary.outside = outside;
    Some(summary)
}

/// `git diff before after [-- path]` as plain text (no colour; the UI colours it), with
/// the untracked files the turn added shown as new files. `path` is repository-relative.
pub fn diff_text(dir: &Path, summary: &ChangeSummary, path: Option<&str>) -> Option<String> {
    // From the repository root, so `path` means the same as in the summary whatever
    // folder the session was started in.
    let root = toplevel(dir)?;
    let mut args = vec!["--literal-pathspecs", "-c", "core.quotepath=false", "diff", "--no-color", "--no-ext-diff", "--no-textconv", "-M", &summary.before, &summary.after];
    // Only what the turn is credited with: the trees also hold other processes' changes.
    let mut mine: Vec<&str> = summary.files.iter().flat_map(|f| std::iter::once(f.path.as_str()).chain(f.old_path.as_deref())).collect();
    mine.retain(|p| path.is_none_or(|want| want == *p));
    let mut text = if mine.is_empty() {
        String::new()
    } else {
        args.push("--");
        args.extend(mine);
        git_text(&root, None, &args)?
    };
    let mut added: Vec<&String> = summary.added_stamps.keys().filter(|p| path.is_none_or(|want| want == p.as_str())).collect();
    added.sort();
    for p in added {
        text.push_str(&new_file_diff(&root, p));
    }
    Some(text)
}

// ------------------------------------------------- new untracked files, shown without git

/// Files bigger than this are not read for line counts or shown in a diff.
const MAX_SHOWN_BYTES: u64 = 1 << 20;

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

fn count_lines(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    (newlines + usize::from(bytes.last().is_some_and(|b| *b != b'\n'))) as u32
}

/// What a new file would show in `git diff`: header and one hunk of added lines. Pure.
fn render_new_file(path: &str, bytes: &[u8], mode: &str) -> String {
    let mut out = format!("diff --git a/{path} b/{path}\nnew file mode {mode}\n");
    if looks_binary(bytes) {
        out.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        return out;
    }
    let text = String::from_utf8_lossy(bytes);
    let n = count_lines(bytes);
    if n == 0 {
        return out;
    }
    out.push_str(&format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1{} @@\n", if n == 1 { String::new() } else { format!(",{n}") }));
    for line in text.split_inclusive('\n') {
        out.push('+');
        out.push_str(line.trim_end_matches('\n'));
        out.push('\n');
    }
    if !bytes.ends_with(b"\n") {
        out.push_str("\\ No newline at end of file\n");
    }
    out
}

/// Added lines of a new untracked file; 0 for binary, unreadable or very large ones.
fn new_file_lines(root: &Path, rel: &str) -> u32 {
    let target = root.join(rel);
    match std::fs::symlink_metadata(&target) {
        Ok(m) if m.is_file() && m.len() <= MAX_SHOWN_BYTES => std::fs::read(&target).ok().filter(|b| !looks_binary(b)).map_or(0, |b| count_lines(&b)),
        _ => 0,
    }
}

/// The diff of one new untracked file, read from disk (never through git's object store).
fn new_file_diff(root: &Path, rel: &str) -> String {
    let target = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&target) else { return String::new() };
    if meta.file_type().is_symlink() {
        let to = std::fs::read_link(&target).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
        return render_new_file(rel, to.as_bytes(), "120000");
    }
    if !meta.is_file() {
        return String::new();
    }
    if meta.len() > MAX_SHOWN_BYTES {
        return format!("diff --git a/{rel} b/{rel}\nnew file ({} bytes, too large to show)\n", meta.len());
    }
    let mode = if is_executable(&meta) { "100755" } else { "100644" };
    std::fs::read(&target).map(|b| render_new_file(rel, &b, mode)).unwrap_or_default()
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

// ------------------------------------------------------------------ cancellation

/// Run blocking `work` on the blocking pool and give its result, or `None` as soon as
/// `cancel` resolves. The work is not stopped: it finishes in the background, and what
/// it does (git writing a temporary index, or reading) is harmless once nobody waits.
async fn until<T: Send + 'static>(cancel: impl Future<Output = ()>, work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let task = tokio::task::spawn_blocking(work);
    tokio::select! {
        done = task => done.ok().flatten(),
        () = cancel => None,
    }
}

/// [`snapshot`] that returns `None` promptly once `cancel` resolves.
pub async fn snapshot_until(dir: PathBuf, cancel: impl Future<Output = ()>) -> Option<Snapshot> {
    until(cancel, move || snapshot(&dir)).await
}

/// [`summarize`] that returns `None` promptly once `cancel` resolves.
pub async fn summarize_until(
    dir: PathBuf,
    before: Snapshot,
    after: Snapshot,
    activity: TurnActivity,
    cancel: impl Future<Output = ()>,
) -> Option<ChangeSummary> {
    until(cancel, move || summarize_turn(&dir, &before, &after, &activity)).await
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
    /// The turn changed these untracked files that already existed. There is no earlier
    /// copy to put back, so nothing was touched.
    UntrackedModified(Vec<String>),
    /// HEAD or the branch moved (during the turn, or since): the turn's changes cannot be
    /// told apart from someone else's, so nothing was touched.
    RepoMoved(String),
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
            Self::UntrackedModified(files) => write!(
                f,
                "not undone: the turn changed {} not tracked by git, so there is no earlier copy to put back: {}. Nothing was touched; \
                 revert {} by hand (files you `git add` before a turn can be undone)",
                if files.len() == 1 { "a file that is" } else { "files that are" },
                files.join(", "),
                if files.len() == 1 { "it" } else { "them" }
            ),
            Self::RepoMoved(what) => write!(
                f,
                "not undone: {what}, so this turn's changes cannot be told apart from changes made by others. Nothing was touched; \
                 revert by hand if you need to"
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
    /// Put the path back to its `before` content; `recreated` is whether the turn deleted it.
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

/// What to do to undo `files`; refuses on the first unsafe path, and on untracked files
/// that cannot be put back, before anything runs.
fn plan_undo(files: &[FileChange]) -> Result<Vec<Step>, UndoError> {
    let mut steps = Vec::new();
    let mut untracked = Vec::new();
    for f in files {
        let paths = std::iter::once(&f.path).chain(f.old_path.as_ref());
        if let Some(bad) = paths.into_iter().find(|p| !is_safe_rel_path(p)) {
            return Err(UndoError::UnsafePath(bad.clone()));
        }
        match (f.status, &f.old_path) {
            (ChangeStatus::Added, _) => steps.push(Step::Delete(f.path.clone())),
            (ChangeStatus::Modified, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: false }),
            (ChangeStatus::Deleted, _) => steps.push(Step::Restore { path: f.path.clone(), recreated: true }),
            (ChangeStatus::ModifiedUntracked, _) => untracked.push(f.path.clone()),
            (ChangeStatus::Renamed, old) => {
                steps.push(Step::Delete(f.path.clone()));
                if let Some(old) = old {
                    steps.push(Step::Restore { path: old.clone(), recreated: true });
                }
            }
        }
    }
    if !untracked.is_empty() {
        return Err(UndoError::UntrackedModified(untracked));
    }
    if steps.is_empty() {
        return Err(UndoError::NothingToUndo);
    }
    Ok(steps)
}

/// The paths an undo would overwrite or delete although someone changed them after the
/// turn. `tree_diff` are the tracked paths that differ between the tree the turn left and
/// the one now; `now` is the untracked manifest now. An untracked file the turn added is
/// fine to delete only while it still looks as the turn left it; any other untracked file
/// sitting at a path the undo is about to write is somebody's.
fn changed_since(steps: &[Step], added: &HashMap<String, Stamp>, tree_diff: &HashSet<&str>, now: &HashMap<String, Stamp>) -> Vec<String> {
    steps
        .iter()
        .map(Step::path)
        .filter(|p| {
            tree_diff.contains(p)
                || match added.get(*p) {
                    Some(left) => now.get(*p) != Some(left),
                    None => now.contains_key(*p),
                }
        })
        .map(String::from)
        .collect()
}

/// A path that is restorable from git output: `<mode> SP <type> SP <hash> TAB <path>\0`.
/// Submodule entries are left out (an undo cannot rebuild them).
fn parse_ls_tree_paths(raw: &str) -> HashSet<String> {
    raw.split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            matches!(meta.split(' ').next()?, "100644" | "100664" | "100755" | "120000").then(|| path.to_string())
        })
        .collect()
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

fn io_err(what: &str, path: &str, e: std::io::Error) -> UndoError {
    UndoError::Failed(format!("{what} {path}: {e}"))
}

/// Something that is not a folder where a folder must be (a file or a link in the way of
/// `rel`'s parents) and that this undo is not itself deleting: git would replace it.
fn blocking_parent(root: &Path, rel: &str, deleting: &HashSet<&str>) -> Option<String> {
    let mut prefix = PathBuf::new();
    let parts: Vec<_> = Path::new(rel).components().collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        prefix.push(part);
        let name = prefix.to_string_lossy();
        if std::fs::symlink_metadata(root.join(&prefix)).is_ok_and(|m| !m.is_dir()) && !deleting.contains(name.as_ref()) {
            return Some(name.into_owned());
        }
    }
    None
}

/// A folder where the file `rel` has to go, holding something the undo does not delete.
fn folder_in_the_way(root: &Path, rel: &str, deleting: &HashSet<&str>) -> bool {
    let under = format!("{rel}/");
    std::fs::symlink_metadata(root.join(rel)).is_ok_and(|m| m.is_dir()) && !deleting.iter().any(|d| d.starts_with(&under))
}

/// Undo what `summary` records, in the repository `dir` belongs to. Blocking.
///
/// Safe by construction: nothing is touched unless every file the turn changed still
/// matches how the turn left it, every path is inside the repository, nothing the turn
/// changed is an untracked file that existed before, and all old contents are known to be
/// in the old tree. Only the paths in the summary are written.
pub fn undo(dir: &Path, summary: &ChangeSummary) -> Result<UndoReport, UndoError> {
    if let Some(m) = &summary.outside.moved {
        return Err(UndoError::RepoMoved(format!("the repository moved during the turn ({})", m.describe())));
    }
    let steps = plan_undo(&summary.files)?;
    let root = toplevel(dir).ok_or_else(|| UndoError::Failed("not inside a git repository".into()))?;
    let root_canon = std::fs::canonicalize(&root).map_err(|e| io_err("cannot resolve", &root.to_string_lossy(), e))?;

    // 1. Has anything the turn touched been edited since? Compare against the state the
    // turn left, not against `before`: only a difference from `after` is someone's edit.
    let now = snapshot(dir).ok_or_else(|| UndoError::Failed("could not read the working tree".into()))?;
    if now.repo != summary.repo_after {
        let m = RepoMove { from: summary.repo_after.clone(), to: now.repo.clone() };
        return Err(UndoError::RepoMoved(format!("the repository moved since the turn ({})", m.describe())));
    }
    let differing = git_text(&root, None, &["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", &summary.after, &now.tree])
        .ok_or_else(|| UndoError::Failed("could not compare the working tree".into()))?;
    let differing: HashSet<&str> = differing.split('\0').filter(|s| !s.is_empty()).collect();
    let changed = changed_since(&steps, &summary.added_stamps, &differing, &now.untracked);
    if !changed.is_empty() {
        return Err(UndoError::ChangedSince(changed));
    }

    // 2. Every old content must be in the old tree, so a failure here leaves the tree untouched.
    let wanted: Vec<&str> = steps.iter().filter(|&s| matches!(s, Step::Restore { .. })).map(|s| s.path()).collect();
    let mut restorable = HashSet::new();
    for chunk in wanted.chunks(200) {
        let mut args = vec!["--literal-pathspecs", "ls-tree", "-z", summary.before.as_str(), "--"];
        args.extend(chunk);
        let listing = git_text(&root, None, &args).ok_or_else(|| UndoError::Failed("could not read the old tree".into()))?;
        restorable.extend(parse_ls_tree_paths(&listing));
    }
    if let Some(path) = wanted.iter().find(|p| !restorable.contains(**p)) {
        return Err(UndoError::Failed(format!("{path} is not in the tree from before the turn (or is a submodule)")));
    }

    // 3. Every target must really be inside the repository and not blocked by something
    // that is not ours to replace.
    let deleting: HashSet<&str> = steps.iter().filter(|&s| matches!(s, Step::Delete(_))).map(|s| s.path()).collect();
    for step in &steps {
        if !inside_root(&root_canon, &root.join(step.path())) {
            return Err(UndoError::UnsafePath(step.path().to_string()));
        }
    }
    for path in &wanted {
        if let Some(p) = blocking_parent(&root, path, &deleting) {
            return Err(UndoError::Failed(format!("{p} is in the way of {path}")));
        }
        if folder_in_the_way(&root, path, &deleting) {
            return Err(UndoError::Failed(format!("{path} is now a folder with other things in it")));
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
    for path in &wanted {
        if folder_in_the_way(&root, path, &HashSet::new()) {
            return Err(UndoError::Failed(format!("{path} is now a folder with other things in it")));
        }
    }
    restore_paths(&root, &summary.before, &wanted)?;
    for step in &steps {
        if let Step::Restore { path, recreated } = step {
            if *recreated { &mut report.recreated } else { &mut report.restored }.push(path.clone());
        }
    }
    Ok(report)
}

/// Put `paths` back as they are in `tree`, through git's own checkout so line endings and
/// clean/smudge filters (git-lfs, `.gitattributes` eol rules) apply as they would for the
/// user. A temporary index holds the tree, so the real index is never touched.
fn restore_paths(root: &Path, tree: &str, paths: &[&str]) -> Result<(), UndoError> {
    if paths.is_empty() {
        return Ok(());
    }
    let tmp = temp_index_path();
    let result = git(root, Some(&tmp), &["read-tree", tree])
        .ok_or_else(|| UndoError::Failed("could not read the old tree".into()))
        .and_then(|_| {
            paths.chunks(200).try_for_each(|chunk| {
                let mut args = vec!["checkout-index", "-f", "--"];
                args.extend(chunk);
                git(root, Some(&tmp), &args).map(drop).ok_or_else(|| UndoError::Failed("git could not put the old files back".into()))
            })
        });
    remove_temp_index(tmp);
    result
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

    use crate::testkit::git_repo::git as run_git;

    /// A repo with a.txt (3 lines), b.txt, c.txt committed and `*.log` ignored.
    fn repo() -> Scratch {
        let s = Scratch::new("repo");
        crate::testkit::git_repo::init_with_commit(
            s.path(),
            &[(".gitignore", "*.log\n"), ("a.txt", "one\ntwo\nthree\n"), ("b.txt", "bee\n"), ("src/c.txt", "see\n")],
        );
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
        let num = "1\t2\ta.txt\x003\t0\tnew.txt\x000\t0\t\0old name.txt\0new name.txt\0-\t-\tt.bin\0";
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
        let raw = "100644 blob aaa\ta.txt\x00100755 blob bbb\trun.sh\x00120000 blob ccc\tlink\x00160000 commit ddd\tsub\0";
        let m = parse_ls_tree_paths(raw);
        assert!(m.contains("a.txt") && m.contains("run.sh") && m.contains("link"));
        assert!(!m.contains("sub"));
    }

    /// A copy that dies halfway: leaves a truncated file behind, then reports the error.
    fn copy_that_fails_halfway(_from: &Path, to: &Path) -> std::io::Result<u64> {
        std::fs::write(to, b"DIRC-truncated")?;
        Err(std::io::Error::other("disk full"))
    }

    #[test]
    fn a_failed_index_copy_leaves_no_partial_file_behind() {
        let s = Scratch::new("copyfail");
        s.write("index", "the real index");
        let tmp = s.path().join("tmp-index");
        assert!(!copy_index_with(&s.path().join("index"), &tmp, copy_that_fails_halfway), "the caller must know the copy failed");
        assert!(!tmp.exists(), "git would read a truncated index as the real one");
    }

    #[test]
    fn a_snapshot_after_a_failed_index_copy_matches_one_after_a_good_copy() {
        let s = repo();
        s.write("a.txt", "edited\n");
        s.write("untracked.txt", "u\n");
        let good = snapshot(s.path()).expect("snapshot");
        let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("a failed copy must not break the snapshot");
        assert_eq!(after_failure.tree, good.tree, "the fallback sees the same tracked files");
        assert_eq!(after_failure.untracked.keys().collect::<Vec<_>>(), good.untracked.keys().collect::<Vec<_>>());
    }

    #[test]
    fn the_fallback_keeps_skip_worktree_entries_of_a_sparse_checkout() {
        let s = repo();
        run_git(s.path(), &["sparse-checkout", "set", "--no-cone", "/a.txt", "/.gitignore", "/src/"]);
        assert!(!s.exists("b.txt"), "b.txt is not checked out");
        let head = run_git(s.path(), &["rev-parse", "HEAD^{tree}"]).trim().to_string();
        let good = snapshot(s.path()).expect("snapshot");
        assert_eq!(good.tree, head, "a file that is not checked out is not a deletion");
        let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("the fallback still snapshots");
        assert_eq!(after_failure.tree, head, "the rebuilt index must keep skip-worktree, or `add -u` records b.txt as deleted");
        s.write("a.txt", "edited\n");
        let edited = snapshot_with(s.path(), copy_that_fails_halfway).expect("snapshot");
        assert_eq!(edited.tree, snapshot(s.path()).unwrap().tree, "and a real edit is still seen");
    }

    #[test]
    fn only_paths_tagged_skip_worktree_are_picked() {
        let tagged = b"H a.txt\0S b.txt\0S dir/with space\0h assumed\0s both\0";
        assert_eq!(skip_worktree_paths(tagged), b"b.txt\0dir/with space\0");
        assert!(skip_worktree_paths(b"").is_empty());
    }

    #[test]
    fn the_fallback_keeps_a_skip_worktree_bit_set_by_hand() {
        // No sparse-checkout patterns for `git add` to fall back on: only the bit says b.txt
        // is not checked out (or edited locally on purpose).
        let s = repo();
        run_git(s.path(), &["update-index", "--skip-worktree", "b.txt"]);
        std::fs::remove_file(s.path().join("b.txt")).unwrap();
        s.write("src/c.txt", "local only\n");
        run_git(s.path(), &["update-index", "--skip-worktree", "src/c.txt"]);
        let good = snapshot(s.path()).expect("snapshot");
        assert_eq!(good.tree, run_git(s.path(), &["rev-parse", "HEAD^{tree}"]).trim());
        let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("the fallback still snapshots");
        assert_eq!(after_failure.tree, good.tree, "b.txt is not deleted and src/c.txt not changed in the snapshot");
    }

    #[test]
    fn the_fallback_matches_a_good_copy_for_an_intent_to_add_file() {
        let s = repo();
        s.write("planned.txt", "later\n");
        run_git(s.path(), &["add", "-N", "planned.txt"]);
        let good = snapshot(s.path()).expect("snapshot");
        let after_failure = snapshot_with(s.path(), copy_that_fails_halfway).expect("snapshot");
        assert_eq!(after_failure.tree, good.tree);
    }

    #[test]
    fn a_snapshot_lists_untracked_files_skips_ignored_ones_and_touches_nothing() {
        let s = repo();
        let status_before = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
        let index_before = std::fs::read(s.path().join(".git/index")).unwrap();
        let t1 = snapshot(s.path()).expect("a tree id");
        assert!(t1.untracked.is_empty());
        s.write("untracked.txt", "u\n");
        s.write("ignored.log", "i\n");
        s.write("deep/er/more.txt", "m\n");
        let t2 = snapshot(s.path()).unwrap();
        assert_eq!(t1.tree, t2.tree, "untracked files are not in the tracked tree");
        let mut names: Vec<_> = t2.untracked.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, ["deep/er/more.txt", "untracked.txt"], "ignored files are left out, paths are repository-relative");
        assert_eq!(t2.untracked["untracked.txt"].size, 2);
        assert!(!run_git(s.path(), &["ls-tree", "-r", "--name-only", &t2.tree]).contains("untracked.txt"));
        // The user's index is as it was and the new file is still untracked, not staged.
        assert_eq!(std::fs::read(s.path().join(".git/index")).unwrap(), index_before);
        let status = run_git(s.path(), &["status", "--porcelain=v1", "--ignored"]);
        assert!(status.contains("?? untracked.txt"), "{status}");
        assert_ne!(status, status_before);
        // Nothing changed, nothing differs.
        assert_eq!(snapshot(s.path()).unwrap(), t2);
        // A tracked edit moves the tree.
        s.write("a.txt", "edited\n");
        assert_ne!(snapshot(s.path()).unwrap().tree, t1.tree);
    }

    #[test]
    fn the_summary_lists_added_modified_and_deleted_files_with_line_counts() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        fake_turn(&s);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!((sum.before.as_str(), sum.after.as_str()), (before.tree.as_str(), after.tree.as_str()));
        let by = files_by_path(&sum);
        assert_eq!(sum.files.len(), 3, "{:?}", sum.files);
        assert_eq!((by["a.txt"].status, by["a.txt"].added, by["a.txt"].removed), (ChangeStatus::Modified, 2, 1));
        assert_eq!((by["new.txt"].status, by["new.txt"].added, by["new.txt"].removed), (ChangeStatus::Added, 3, 0));
        assert_eq!((by["b.txt"].status, by["b.txt"].added, by["b.txt"].removed), (ChangeStatus::Deleted, 0, 1));
        assert!(!by.contains_key("debug.log"), "ignored files are not changes");
        assert_eq!(sum.totals(), (5, 2));

        let diff = diff_text(s.path(), &sum, None).unwrap();
        assert!(diff.contains("+TWO") && diff.contains("-two") && diff.contains("+++ b/new.txt") && diff.contains("--- a/b.txt"), "{diff}");
        assert!(!diff.contains('\u{1b}'), "no colour codes");
        let one = diff_text(s.path(), &sum, Some("new.txt")).unwrap();
        assert!(one.contains("new.txt") && !one.contains("a.txt"), "{one}");
        // A glob is a name here, not a pattern.
        assert_eq!(diff_text(s.path(), &sum, Some("*.txt")).unwrap(), "");
    }

    #[test]
    fn nothing_changed_gives_an_empty_summary() {
        let s = repo();
        let t = snapshot(s.path()).unwrap();
        assert!(summarize(s.path(), &t, &snapshot(s.path()).unwrap()).unwrap().is_empty());
    }

    #[test]
    fn a_plain_move_is_a_deleted_and_an_added_file_and_undo_moves_it_back() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        std::fs::rename(s.path().join("a.txt"), s.path().join("renamed.txt")).unwrap();
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        let by = files_by_path(&sum);
        assert_eq!(sum.files.len(), 2, "{:?}", sum.files);
        assert_eq!((by["a.txt"].status, by["renamed.txt"].status), (ChangeStatus::Deleted, ChangeStatus::Added));
        assert_eq!(by["renamed.txt"].added, 3);
        undo(s.path(), &sum).unwrap();
        assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
        assert!(!s.exists("renamed.txt"));
    }

    #[test]
    fn a_git_mv_is_a_rename_and_undo_moves_it_back() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        run_git(s.path(), &["mv", "a.txt", "renamed.txt"]);
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
        // (new.txt is untracked, so rewriting it, even with the same text, changes its
        // mtime and counts as an edit: we cannot compare content we never hashed.)
        s.write("new.txt", "x\ny\nz\n");
        s.write("b.txt", "back again\n");
        s.write("a.txt", "edited again\n");
        match undo(s.path(), &sum).unwrap_err() {
            UndoError::ChangedSince(mut files) => {
                files.sort();
                assert_eq!(files, ["a.txt", "b.txt", "new.txt"]);
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
            before: t.tree.clone(),
            after: t.tree,
            added_stamps: HashMap::new(),
            ..ChangeSummary::default()
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
        let snap = |tree: String| Snapshot { tree, untracked: HashMap::new(), taken_ns: 0, repo: RepoState::default() };
        assert_eq!(summarize(s.path(), &snap(t.clone()), &snap("1".repeat(40))), None);
        let sum = ChangeSummary {
            files: vec![FileChange { path: "f.txt".into(), status: ChangeStatus::Added, added: 1, removed: 0, old_path: None }],
            before: t.clone(),
            after: t,
            added_stamps: HashMap::new(),
            ..ChangeSummary::default()
        };
        assert_eq!(diff_text(s.path(), &sum, None), None);
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

    // ------------------------------------------------------------ untracked files

    /// `git hash-object` WITHOUT `-w`: the id a blob with this content would have.
    /// The blob id git would give `content`, computed WITHOUT writing it (`hash-object`
    /// with no `-w`) from a temp file outside the repo, via the shared `run_git` helper,
    /// which fails with git's own message if anything goes wrong.
    fn blob_id(dir: &Path, content: &str) -> String {
        let tmp = std::env::temp_dir().join(format!("jev-blob-{}-{:x}", std::process::id(), content.len() as u64 ^ 0x9e37_79b9));
        std::fs::write(&tmp, content).unwrap();
        let id = run_git(dir, &["hash-object", "--no-filters", tmp.to_str().unwrap()]).trim().to_string();
        let _ = std::fs::remove_file(&tmp);
        id
    }

    fn object_exists(dir: &Path, id: &str) -> bool {
        run_git(dir, &["cat-file", "--batch-all-objects", "--batch-check"]).lines().any(|l| l.starts_with(id))
    }

    #[test]
    fn untracked_files_are_never_written_into_the_object_store() {
        let s = repo();
        let secret = "API_KEY=unique-secret-7f3a9c1e-not-in-git\n";
        let id = blob_id(s.path(), secret);
        assert!(!object_exists(s.path(), &id), "the check itself: the blob is not there yet");
        s.write("secret.env", secret);
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "one\nTWO\nthree\n");
        s.write("data/big.bin", "dataset\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(sum.files.len(), 2, "{:?}", sum.files);
        diff_text(s.path(), &sum, None).unwrap();
        undo(s.path(), &sum).unwrap();
        assert!(!object_exists(s.path(), &id), "the untracked secret must not be hashed into .git/objects");
        assert!(!object_exists(s.path(), &blob_id(s.path(), "dataset\n")));
        assert!(s.exists("secret.env"), "it was there before the turn, so undo leaves it");
    }

    /// Write `text` and give the file a fixed mtime, so two writes never look alike by luck.
    /// Write `rel` with its mtime `secs` after `base`. Each test takes its own `base` when it
    /// starts, just before its snapshots: deterministic stamps that still count as "made
    /// during the turn" (a base shared by the whole test process drifts past the 2 s slack
    /// once the suite has been running a while).
    fn write_at(s: &Scratch, base: std::time::SystemTime, rel: &str, text: &str, secs: u64) {
        s.write(rel, text);
        let f = std::fs::OpenOptions::new().write(true).open(s.path().join(rel)).unwrap();
        f.set_modified(base + std::time::Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn a_changed_untracked_file_is_reported_and_undo_refuses_to_touch_anything() {
        let s = repo();
        let t0 = std::time::SystemTime::now();
        write_at(&s, t0, "notes.txt", "mine\n", 0);
        write_at(&s, t0, "same.txt", "stays\n", 0);
        write_at(&s, t0, "gone.txt", "bye\n", 0);
        let before = snapshot(s.path()).unwrap();
        // Same size, different mtime still counts; so does a delete.
        write_at(&s, t0, "notes.txt", "MINE\n", 5);
        std::fs::remove_file(s.path().join("gone.txt")).unwrap();
        s.write("a.txt", "one\nTWO\nthree\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        let by = files_by_path(&sum);
        assert_eq!(sum.files.len(), 3, "an untouched untracked file is not a change: {:?}", sum.files);
        assert_eq!(by["a.txt"].status, ChangeStatus::Modified);
        assert_eq!(by["notes.txt"].status, ChangeStatus::ModifiedUntracked);
        assert_eq!(by["gone.txt"].status, ChangeStatus::ModifiedUntracked);

        let err = undo(s.path(), &sum).unwrap_err();
        assert_eq!(err, UndoError::UntrackedModified(vec!["gone.txt".into(), "notes.txt".into()]));
        let msg = err.to_string();
        assert!(msg.contains("notes.txt") && msg.contains("gone.txt") && msg.contains("no earlier copy"), "{msg}");
        assert_eq!(s.read("notes.txt"), "MINE\n");
        assert_eq!(s.read("a.txt"), "one\nTWO\nthree\n", "nothing else was undone either");
    }

    #[test]
    fn an_untracked_file_the_turn_added_is_deleted_by_undo_only_while_it_is_as_the_turn_left_it() {
        let s = repo();
        let t0 = std::time::SystemTime::now();
        let before = snapshot(s.path()).unwrap();
        write_at(&s, t0, "new.txt", "made by the turn\n", 0);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(sum.files.len(), 1);
        assert_eq!((sum.files[0].status, sum.files[0].added), (ChangeStatus::Added, 1));
        assert!(sum.added_stamps.contains_key("new.txt"));

        write_at(&s, t0, "new.txt", "made by the turn\n", 9); // touched by someone since
        assert_eq!(undo(s.path(), &sum), Err(UndoError::ChangedSince(vec!["new.txt".into()])));
        assert!(s.exists("new.txt"));
        write_at(&s, t0, "new.txt", "made by the turn\n", 0);
        assert_eq!(undo(s.path(), &sum).unwrap().removed, ["new.txt"]);
        assert!(!s.exists("new.txt"));
    }

    #[test]
    fn an_untracked_file_the_user_recreated_at_a_deleted_path_is_not_overwritten() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        run_git(s.path(), &["rm", "-q", "b.txt"]); // the turn deletes it from the index too
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        s.write("b.txt", "the user wrote this meanwhile\n"); // untracked now
        assert_eq!(undo(s.path(), &sum), Err(UndoError::ChangedSince(vec!["b.txt".into()])));
        assert_eq!(s.read("b.txt"), "the user wrote this meanwhile\n");
    }

    #[test]
    fn a_pre_existing_untracked_file_the_turn_only_staged_is_not_an_addition_to_delete() {
        let s = repo();
        let t0 = std::time::SystemTime::now();
        write_at(&s, t0, "notes.txt", "mine\n", 0);
        let before = snapshot(s.path()).unwrap();
        run_git(s.path(), &["add", "notes.txt"]);
        s.write("a.txt", "one\nTWO\nthree\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(sum.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
        undo(s.path(), &sum).unwrap();
        assert_eq!(s.read("notes.txt"), "mine\n", "it was there before the turn");
    }

    #[test]
    fn a_tracked_file_the_turn_only_removed_from_the_index_is_still_there_and_not_a_change() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        run_git(s.path(), &["rm", "-q", "--cached", "b.txt"]);
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        assert!(sum.is_empty(), "{:?}", sum.files);
        assert!(s.exists("b.txt"));
    }

    #[test]
    fn untracked_changes_merge_into_the_file_list() {
        let st = |size| Stamp { size, mtime_ns: 1 };
        let snap = |tree: &str, files: &[(&str, u64)]| Snapshot { tree: tree.into(), untracked: files.iter().map(|(p, n)| (p.to_string(), st(*n))).collect(), taken_ns: 0, repo: RepoState::default() };
        let fc = |p: &str, status| FileChange::new(p.into(), status);
        let before = snap("t1", &[("keep", 1), ("edit", 1), ("gone", 1), ("staged", 1), ("moved", 1)]);
        let after = snap("t2", &[("keep", 1), ("edit", 2), ("new", 3), ("zz", 1), ("untracked-now", 1)]);
        let tree = vec![
            fc("staged", ChangeStatus::Added),
            fc("untracked-now", ChangeStatus::Deleted),
            fc("tracked", ChangeStatus::Modified),
            FileChange { old_path: Some("old".into()), ..fc("moved", ChangeStatus::Renamed) },
        ];
        // `staged` is on disk unchanged; `gone` and `moved` are not.
        let on_disk = |p: &str| (p == "staged").then(|| st(1));
        let m = merge_untracked(tree, &before, &after, on_disk);
        let got: Vec<_> = m.files.iter().map(|f| (f.path.as_str(), f.status)).collect();
        assert_eq!(
            got,
            [
                ("tracked", ChangeStatus::Modified),
                ("old", ChangeStatus::Deleted),
                ("edit", ChangeStatus::ModifiedUntracked),
                ("gone", ChangeStatus::ModifiedUntracked),
                ("moved", ChangeStatus::ModifiedUntracked),
                ("new", ChangeStatus::Added),
                ("zz", ChangeStatus::Added),
            ]
        );
        assert_eq!(m.added.len(), 2);
    }

    #[test]
    fn the_edited_since_rule_covers_added_untracked_tracked_and_recreated_paths() {
        let st = |n| Stamp { size: n, mtime_ns: 0 };
        let steps = vec![
            Step::Delete("same".into()),
            Step::Delete("touched".into()),
            Step::Delete("missing".into()),
            Step::Restore { path: "tracked-edit".into(), recreated: false },
            Step::Restore { path: "recreated".into(), recreated: true },
            Step::Restore { path: "fine".into(), recreated: true },
        ];
        let added: HashMap<_, _> = [("same", st(1)), ("touched", st(1)), ("missing", st(1))].map(|(p, s)| (p.to_string(), s)).into();
        let now: HashMap<_, _> = [("same", st(1)), ("touched", st(2)), ("recreated", st(1))].map(|(p, s)| (p.to_string(), s)).into();
        let diff: HashSet<&str> = ["tracked-edit"].into();
        assert_eq!(changed_since(&steps, &added, &diff, &now), ["touched", "missing", "tracked-edit", "recreated"]);
    }

    #[test]
    fn a_new_file_renders_like_git_diff_would() {
        assert_eq!(
            render_new_file("a b.txt", b"x\ny\n", "100644"),
            "diff --git a/a b.txt b/a b.txt\nnew file mode 100644\n--- /dev/null\n+++ b/a b.txt\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
        assert!(render_new_file("f", b"one", "100755").ends_with("@@ -0,0 +1 @@\n+one\n\\ No newline at end of file\n"));
        assert!(render_new_file("f", b"\0\x01", "100644").contains("Binary files /dev/null and b/f differ"));
        assert!(!render_new_file("empty", b"", "100644").contains("@@"));
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b""), 0);
    }

    #[test]
    fn the_diff_shows_new_untracked_files_without_git_hashing_them() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("new.txt", "x\ny\nz\n");
        s.write("a.txt", "one\nTWO\nthree\n");
        let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
        let all = diff_text(s.path(), &sum, None).unwrap();
        assert!(all.contains("+TWO") && all.contains("+++ b/new.txt") && all.contains("+y\n"), "{all}");
        let one = diff_text(s.path(), &sum, Some("new.txt")).unwrap();
        assert!(one.contains("+++ b/new.txt") && !one.contains("a.txt"), "{one}");
        let other = diff_text(s.path(), &sum, Some("a.txt")).unwrap();
        assert!(other.contains("+TWO") && !other.contains("new.txt"), "{other}");
        assert!(!object_exists(s.path(), &blob_id(s.path(), "x\ny\nz\n")));
    }

    // ------------------------------------------------------------ diff from a subfolder

    #[test]
    fn diff_of_one_file_works_when_the_project_is_a_subfolder_of_the_repository() {
        let s = repo();
        let sub = s.path().join("src");
        let before = snapshot(&sub).unwrap();
        fake_turn(&s);
        s.write("src/c.txt", "see\nmore\n");
        let sum = summarize(&sub, &before, &snapshot(&sub).unwrap()).unwrap();
        // The summary names files from the repository root, and so does the diff.
        for path in ["a.txt", "src/c.txt", "new.txt"] {
            assert!(sum.files.iter().any(|f| f.path == path), "{path} in {:?}", sum.files);
            let d = diff_text(&sub, &sum, Some(path)).unwrap();
            assert!(d.contains(&format!("+++ b/{path}")), "{path}: {d}");
        }
        assert!(diff_text(&sub, &sum, Some("a.txt")).unwrap().contains("+TWO"));
    }

    // ------------------------------------------------------------ eol and filters

    /// Commit `rel` (written as `text`), run a turn that changes it, undo; give the bytes after.
    fn commit_change_undo(s: &Scratch, rel: &str, text: &str, changed: &str) -> Vec<u8> {
        std::fs::write(s.path().join(rel), text).unwrap();
        run_git(s.path(), &["add", "-A"]);
        run_git(s.path(), &["commit", "-q", "-m", "add"]);
        let before = snapshot(s.path()).unwrap();
        std::fs::write(s.path().join(rel), changed).unwrap();
        let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
        undo(s.path(), &sum).unwrap_or_else(|e| panic!("{e}"));
        std::fs::read(s.path().join(rel)).unwrap()
    }

    #[test]
    fn undo_gives_back_crlf_files_under_autocrlf() {
        let s = repo();
        run_git(s.path(), &["config", "core.autocrlf", "true"]);
        let original = "line one\r\nline two\r\n";
        let restored = commit_change_undo(&s, "win.txt", original, "line one\r\nCHANGED\r\n");
        assert_eq!(restored, original.as_bytes(), "byte-identical to the file the user had");
        assert!(run_git(s.path(), &["status", "--porcelain"]).trim().is_empty());
    }

    #[test]
    fn undo_applies_gitattributes_eol_rules() {
        let s = repo();
        s.write(".gitattributes", "win.dat text eol=crlf\n");
        let original = "one\r\ntwo\r\n";
        let restored = commit_change_undo(&s, "win.dat", original, "one\r\nCHANGED\r\n");
        assert_eq!(restored, original.as_bytes());
    }

    #[test]
    fn undo_runs_the_smudge_filter_so_filtered_files_come_back_whole() {
        let s = repo();
        // The same code path git-lfs takes: the stored blob is the clean form.
        run_git(s.path(), &["config", "filter.up.clean", "sed s/secret/XXX/"]);
        run_git(s.path(), &["config", "filter.up.smudge", "sed s/XXX/secret/"]);
        s.write(".gitattributes", "f.dat filter=up\n");
        let original = "my secret value\n";
        let restored = commit_change_undo(&s, "f.dat", original, "something else\n");
        assert_eq!(String::from_utf8(restored).unwrap(), original, "not the clean form (`XXX`)");
        assert_eq!(run_git(s.path(), &["show", "HEAD:f.dat"]), "my XXX value\n", "the repository itself holds the clean form");
    }

    #[test]
    fn restoring_leaves_the_real_index_alone() {
        let s = repo();
        run_git(s.path(), &["config", "core.autocrlf", "true"]);
        std::fs::write(s.path().join("w.txt"), "x\r\ny\r\n").unwrap();
        run_git(s.path(), &["add", "-A"]);
        run_git(s.path(), &["commit", "-q", "-m", "w"]);
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "changed\n");
        std::fs::write(s.path().join("w.txt"), "z\r\n").unwrap();
        let sum = summarize(s.path(), &before, &snapshot(s.path()).unwrap()).unwrap();
        run_git(s.path(), &["add", "a.txt"]); // the user stages work after the turn
        let index_before = std::fs::read(s.path().join(".git/index")).unwrap();
        // a.txt is staged now, but its working copy is still as the turn left it.
        undo(s.path(), &sum).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(std::fs::read(s.path().join(".git/index")).unwrap(), index_before);
        assert_eq!(std::fs::read(s.path().join("w.txt")).unwrap(), b"x\r\ny\r\n");
    }

    // ------------------------------------------------------------ cancellation

    #[tokio::test]
    async fn a_cancel_returns_at_once_while_a_slow_snapshot_finishes_in_the_background() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let out = until(tokio::time::sleep(Duration::from_millis(20)), || {
            std::thread::sleep(Duration::from_millis(1000));
            Some("late")
        })
        .await;
        assert_eq!(out, None);
        assert!(started.elapsed() < Duration::from_millis(200), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn without_a_cancel_the_blocking_result_comes_back() {
        assert_eq!(until(std::future::pending::<()>(), || Some(7)).await, Some(7));
        assert_eq!(until(std::future::pending::<()>(), || None::<u8>).await, None);
        let s = repo();
        let snap = snapshot_until(s.path().to_path_buf(), std::future::pending()).await.expect("a snapshot");
        let again = snapshot_until(s.path().to_path_buf(), std::future::pending()).await.unwrap();
        let sum = summarize_until(s.path().to_path_buf(), snap, again, TurnActivity::default(), std::future::pending()).await.unwrap();
        assert!(sum.is_empty());
    }

    #[test]
    fn a_hidden_file_revealed_by_a_gitignore_edit_is_never_deleted_by_undo() {
        let s = repo();
        // An ignored file the user had long before the turn (say a secret).
        s.write("secret.log", "TOKEN=abc\n");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        std::fs::File::options().write(true).open(s.path().join("secret.log")).unwrap().set_modified(old).unwrap();
        let before = snapshot(s.path()).unwrap();
        // The turn un-ignores it by rewriting .gitignore, and adds a genuinely new file.
        s.write(".gitignore", "# nothing ignored now\n");
        s.write("made.txt", "new\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize(s.path(), &before, &after).unwrap();
        let by = files_by_path(&sum);
        assert_eq!(by["secret.log"].status, ChangeStatus::ModifiedUntracked, "it existed before: not 'added'");
        assert!(!sum.added_stamps.contains_key("secret.log"));
        assert_eq!(by["made.txt"].status, ChangeStatus::Added);
        // Undo refuses as a whole rather than deleting the user's file.
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::UntrackedModified(_))));
        assert_eq!(std::fs::read_to_string(s.path().join("secret.log")).unwrap(), "TOKEN=abc\n");
    }

    #[test]
    fn a_same_size_edit_in_the_same_second_as_the_commit_is_never_missed() {
        std::thread::scope(|scope| {
            for worker in 0..4 {
                scope.spawn(move || {
                    for _ in (worker..30).step_by(4) {
                        let s = repo();
                        let before = snapshot(s.path()).unwrap();
                        s.write("a.txt", "ONE\ntwo\nthree\n");
                        let after = snapshot(s.path()).unwrap();
                        assert_ne!(before.tree, after.tree, "the edit must be seen");
                        let sum = summarize(s.path(), &before, &after).unwrap();
                        assert_eq!(sum.files.len(), 1);
                        undo(s.path(), &sum).unwrap();
                        assert_eq!(std::fs::read_to_string(s.path().join("a.txt")).unwrap(), "one\ntwo\nthree\n");
                    }
                });
            }
        });
    }

    // ------------------------------------------------ changes by other processes

    fn tools(lines: &[&str]) -> TurnActivity {
        TurnActivity::from_tools(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn tool_lines_are_classified_into_written_paths_and_opaque_runs() {
        let read_only = tools(&["Grep foo", "Read /r/a.rs", "Glob *.md"]);
        assert!(read_only.written.is_empty() && !read_only.opaque && read_only.reported);
        assert!(!TurnActivity::default().reported);
        let a = tools(&["Edit /r/a.rs", "edit /r/b c.rs", "MultiEdit src/x.rs"]);
        assert_eq!(a.written, ["/r/a.rs", "/r/b c.rs", "src/x.rs"]);
        assert!(!a.opaque);
        for opaque in ["Bash cargo test", "shell ls", "Task", "NotebookEdit", "Edit", "mcp__jev__delegate x"] {
            assert!(tools(&[opaque]).opaque, "{opaque}");
        }
        assert!(tools(&[&format!("Edit /{}", "x".repeat(150))]).opaque, "a cut-off path cannot be matched");
    }

    #[test]
    fn a_branch_switched_by_someone_else_during_a_read_only_turn_is_not_the_turns() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        // another process: new branch with a commit that touches several files, checked out
        run_git(s.path(), &["checkout", "-q", "-b", "other"]);
        s.write("a.txt", "merged\n");
        s.write("docs/new.md", "doc\n");
        run_git(s.path(), &["add", "-A"]);
        run_git(s.path(), &["commit", "-q", "-m", "other"]);
        let after = snapshot(s.path()).unwrap();
        let raw = summarize(s.path(), &before, &after).unwrap();
        assert_eq!(raw.files.len(), 2, "the plain diff blames the turn for both");

        let sum = summarize_turn(s.path(), &before, &after, &tools(&["Grep foo"])).unwrap();
        assert!(sum.files.is_empty(), "{:?}", sum.files);
        assert_eq!(sum.outside.files.len(), 2);
        let moved = sum.outside.moved.as_ref().expect("the move is reported");
        assert!(moved.describe().contains("→ other"), "{}", moved.describe());
        let warning = sum.outside.warning().unwrap();
        assert!(warning.contains("2 files changed outside this turn") && warning.contains("not included"), "{warning}");
        assert!(!sum.is_empty(), "the warning must survive to the end of the turn");
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
        assert_eq!(s.read("a.txt"), "merged\n", "nothing was reverted");
        assert_eq!(diff_text(s.path(), &sum, None).as_deref(), Some(""), "/diff shows none of it");
    }

    #[test]
    fn a_moved_head_refuses_undo_even_for_files_the_turn_wrote() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "mine\n");
        run_git(s.path(), &["commit", "-q", "-am", "someone commits"]);
        let after = snapshot(s.path()).unwrap();
        let abs = s.path().join("a.txt");
        let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display())])).unwrap();
        assert_eq!(sum.files.len(), 1, "the edit is still shown");
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
        assert_eq!(s.read("a.txt"), "mine\n");
    }

    #[test]
    fn a_doc_written_by_someone_else_during_a_read_only_turn_is_outside() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("docs/notes.md", "from another session\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize_turn(s.path(), &before, &after, &tools(&["Grep todo", "Read a.txt"])).unwrap();
        assert!(sum.files.is_empty());
        assert_eq!(sum.outside.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["docs/notes.md"]);
        assert!(sum.outside.moved.is_none() && !sum.outside.uncertain);
        assert!(sum.outside.warning().unwrap().contains("1 file changed outside this turn"));
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::NothingToUndo)));
        assert_eq!(s.read("docs/notes.md"), "from another session\n");
    }

    #[test]
    fn only_the_files_the_edit_tools_named_are_attributed_and_undone() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "one\nTWO\nthree\n");
        s.write("theirs.txt", "not mine\n");
        std::fs::write(s.path().join("b.txt"), "someone else edits b\n").unwrap();
        let after = snapshot(s.path()).unwrap();
        let abs = s.path().join("a.txt");
        let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display()), "Read b.txt"])).unwrap();
        assert_eq!(sum.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
        let mut outside: Vec<&str> = sum.outside.files.iter().map(|f| f.path.as_str()).collect();
        outside.sort();
        assert_eq!(outside, ["b.txt", "theirs.txt"]);
        let diff = diff_text(s.path(), &sum, None).unwrap();
        assert!(diff.contains("a.txt") && !diff.contains("b.txt"), "{diff}");
        undo(s.path(), &sum).unwrap();
        assert_eq!(s.read("a.txt"), "one\ntwo\nthree\n");
        assert_eq!(s.read("b.txt"), "someone else edits b\n");
        assert_eq!(s.read("theirs.txt"), "not mine\n");
        // a relative path from the turn's own folder matches the same way
        let rel = summarize_turn(s.path(), &before, &after, &tools(&["Edit a.txt"])).unwrap();
        assert_eq!(rel.files.len(), 1);
    }

    #[test]
    fn a_turn_that_reported_no_tools_keeps_its_changes_marked_uncertain() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "edited without a tool line\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize_turn(s.path(), &before, &after, &TurnActivity::default()).unwrap();
        assert_eq!(sum.files.len(), 1, "unreported edits are not hidden");
        assert!(sum.outside.files.is_empty() && sum.outside.uncertain);
    }

    #[test]
    fn after_a_shell_command_every_change_is_kept_but_marked_uncertain() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("generated.txt", "by a script\n");
        let after = snapshot(s.path()).unwrap();
        let sum = summarize_turn(s.path(), &before, &after, &tools(&["Bash ./gen.sh"])).unwrap();
        assert_eq!(sum.files.len(), 1);
        assert!(sum.outside.files.is_empty() && sum.outside.uncertain);
        assert!(sum.outside.warning().unwrap().contains("shell command ran"));
        // every file named by an edit tool: nothing to doubt
        let abs = s.path().join("generated.txt");
        let named = summarize_turn(s.path(), &before, &after, &tools(&["Bash ls", &format!("Write {}", abs.display())])).unwrap();
        assert!(!named.outside.uncertain);
    }

    #[test]
    fn undo_refuses_when_the_branch_changed_after_the_turn() {
        let s = repo();
        let before = snapshot(s.path()).unwrap();
        s.write("a.txt", "one\nTWO\nthree\n");
        let after = snapshot(s.path()).unwrap();
        let abs = s.path().join("a.txt");
        let sum = summarize_turn(s.path(), &before, &after, &tools(&[&format!("Edit {}", abs.display())])).unwrap();
        run_git(s.path(), &["checkout", "-q", "-b", "later"]);
        assert!(matches!(undo(s.path(), &sum), Err(UndoError::RepoMoved(_))));
        assert_eq!(s.read("a.txt"), "one\nTWO\nthree\n");
        run_git(s.path(), &["checkout", "-q", "-"]);
        undo(s.path(), &sum).unwrap();
    }
}
