//! Running `git` and reading its output. The parsers are pure and tested on values.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::changes::domain::file_change::{ChangeStatus, FileChange};

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
pub(in crate::changes) fn build_files(name_status: &str, numstat: &str) -> Vec<FileChange> {
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
pub(in crate::changes) fn parse_untracked_list(raw: &str) -> Vec<&str> {
    raw.split('\0').filter(|p| !p.is_empty() && !p.ends_with('/') && !p.contains('\u{FFFD}')).collect()
}

/// Run `git -C dir args...` (optionally on another index) and return stdout.
pub(in crate::changes) fn git(dir: &Path, index: Option<&Path>, args: &[&str]) -> Option<Vec<u8>> {
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

pub(in crate::changes) fn git_text(dir: &Path, index: Option<&Path>, args: &[&str]) -> Option<String> {
    git(dir, index, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A tree id looks like a hex object name (40 chars, or 64 with sha256).
pub(in crate::changes) fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A path that is restorable from git output: `<mode> SP <type> SP <hash> TAB <path>\0`.
/// Submodule entries are left out (an undo cannot rebuild them).
pub(in crate::changes) fn parse_ls_tree_paths(raw: &str) -> HashSet<String> {
    raw.split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            matches!(meta.split(' ').next()?, "100644" | "100664" | "100755" | "120000").then(|| path.to_string())
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

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
    fn ls_tree_parses_modes_and_skips_submodules() {
        let raw = "100644 blob aaa\ta.txt\x00100755 blob bbb\trun.sh\x00120000 blob ccc\tlink\x00160000 commit ddd\tsub\0";
        let m = parse_ls_tree_paths(raw);
        assert!(m.contains("a.txt") && m.contains("run.sh") && m.contains("link"));
        assert!(!m.contains("sub"));
    }
}
