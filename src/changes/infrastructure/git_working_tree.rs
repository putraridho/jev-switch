//! What one turn changed in the working tree: a summary and the diff, read through `git`.
//!
//! A `Snapshot` has two parts, taken before and after the turn:
//!
//! * the **tracked** content, as a git *tree object* built through a temporary index
//!   (a copy of the real one, plus `git add -u`). Only files the user already gave to git
//!   are ever hashed into `.git/objects`; summary, diff and undo are complete for them.
//! * a cheap **manifest** of the untracked, non-ignored files (path, size, mtime; no
//!   content is read). A file that is untracked now and was not in the manifest was added
//!   by the turn, so undo may delete it. One that was there and changed (or vanished) is
//!   `ChangeStatus::ModifiedUntracked`: we never had its old content, so undo refuses.
//!
//! Ignored files are never looked at, so an undo cannot reach them. The user's index,
//! branch and working tree are never touched by a snapshot. Everything here is best
//! effort: no git, not a repository or any git failure gives `None`, and a turn never
//! fails because of it.
//!
//! The pure parts live in `changes::domain` (types, the manifest merge, attribution, the
//! undo plan), the parsers in [`super::git_output`], the temporary index in
//! [`super::git_temp_index`]; this file is the thin part that runs `git` and looks at files.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::changes::domain::change_set::{attribute, ChangeSummary, OutsideChanges};
use crate::changes::domain::file_change::ChangeStatus;
use crate::changes::domain::new_file_diff::{count_lines, looks_binary, render_new_file, MAX_SHOWN_BYTES};
use crate::changes::domain::snapshot::{RepoMove, RepoState, Snapshot, Stamp};
use crate::changes::domain::turn_activity::TurnActivity;
use crate::changes::domain::untracked_merge::{merge_untracked, Merged};
use crate::changes::infrastructure::git_output::{build_files, git, git_text, is_object_id, parse_untracked_list};
use crate::changes::infrastructure::git_temp_index::{copy_index_with, remove_temp_index, seed_index_from_entries, temp_index_path};

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

pub(in crate::changes) fn toplevel(dir: &Path) -> Option<PathBuf> {
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
pub(super) fn snapshot_with(dir: &Path, copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>) -> Option<Snapshot> {
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
