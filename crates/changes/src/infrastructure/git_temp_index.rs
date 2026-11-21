//! A temporary copy of the index, so hashing the working tree never touches the real one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use crate::infrastructure::git_output::git;

pub(super) fn temp_index_path() -> PathBuf {
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
pub(super) fn copy_index_with(real: &Path, tmp: &Path, copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>) -> bool {
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
pub(super) fn seed_index_from_entries(root: &Path, tmp: &Path) -> Option<()> {
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
pub(super) fn skip_worktree_paths(tagged: &[u8]) -> Vec<u8> {
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
        .args(jev_shared::infrastructure::git_guard::no_repo_programs(root))
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

pub(super) fn remove_temp_index(tmp: PathBuf) {
    let _ = std::fs::remove_file(&tmp);
    let mut lock = tmp.into_os_string();
    lock.push(".lock");
    let _ = std::fs::remove_file(lock);
}
