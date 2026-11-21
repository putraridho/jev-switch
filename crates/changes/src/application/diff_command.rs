//! What `/diff` and `/rewind` need beyond one turn's diff: the scope (the last turn, one
//! earlier checkpoint, or everything this session did), reverting a single hunk, and
//! exporting a patch. Pure except [`revert_hunk`] and [`export_patch`], which
//! run `git apply` and write a file, and are tested on real temp repositories.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::application::ports::WorkingTree;
use crate::domain::change_set::ChangeSummary;
use crate::domain::diff_parse::FileDiff;

/// Which changes to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The last turn.
    Last,
    /// The nth turn of this session that changed files (1-based).
    Turn(usize),
    /// Everything this session changed, as one net diff.
    All,
}

/// The summaries of two turns as one: from where the first started to where the last
/// ended, over every file either touched. `None` for no turns.
pub fn merge(history: &[ChangeSummary]) -> Option<ChangeSummary> {
    let (first, last) = (history.first()?, history.last()?);
    let mut merged = ChangeSummary { before: first.before.clone(), after: last.after.clone(), repo_after: last.repo_after.clone(), ..ChangeSummary::default() };
    for s in history {
        for f in &s.files {
            if !merged.files.iter().any(|m| m.path == f.path) {
                merged.files.push(f.clone());
            }
        }
        merged.added_stamps.extend(s.added_stamps.clone());
    }
    Some(merged)
}

/// The summary `scope` names, or what to tell the user.
pub fn summary_for(scope: Scope, last: Option<&ChangeSummary>, history: &[ChangeSummary]) -> Result<ChangeSummary, String> {
    match scope {
        Scope::Last => last.cloned().ok_or_else(|| "the last turn changed no files (or this folder is not a git repository)".to_string()),
        Scope::Turn(n) => history
            .get(n - 1)
            .cloned()
            .ok_or_else(|| format!("there is no turn {n}: this session has {} turn(s) that changed files (see /diff --list)", history.len())),
        Scope::All => merge(history).ok_or_else(|| "this session has not changed any files yet".to_string()),
    }
}

fn git_apply(tree: &dyn WorkingTree, root: &Path, patch: &str, extra: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(tree.no_repo_programs(root)).args(["apply", "--whitespace=nowarn"]).args(extra);
    cmd.env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE");
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(patch.as_bytes()).map_err(|e| format!("could not hand the hunk to git: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("git failed: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr);
        Err(why.lines().next().unwrap_or("git could not apply it").trim().to_string())
    }
}

/// Take hunk `index` of `file` back out of the working tree (`git apply -R` of just that
/// hunk). Checked first, so a hunk the file no longer matches changes nothing. The index
/// and every other hunk stay as they are.
pub fn revert_hunk(tree: &dyn WorkingTree, dir: &Path, file: &FileDiff, index: usize) -> Result<(), String> {
    let patch = file.hunk_patch(index).ok_or_else(|| format!("{} has no hunk {}", file.path, index + 1))?;
    let root = tree.repo_root(dir).ok_or_else(|| "not inside a git repository".to_string())?;
    git_apply(tree, &root, &patch, &["-R", "--check"]).map_err(|e| format!("{} changed since: {e}", file.path))?;
    git_apply(tree, &root, &patch, &["-R"])
}

/// Write the diff as a patch file `git apply` or `patch -p1` takes. Relative paths are
/// from `dir`; an existing file is not overwritten.
pub fn export_patch(dir: &Path, files: &[FileDiff], dest: &str) -> Result<std::path::PathBuf, String> {
    let target = dir.join(dest);
    if target.exists() {
        return Err(format!("{} already exists; pick another name", target.display()));
    }
    std::fs::write(&target, crate::domain::diff_parse::to_patch(files)).map_err(|e| format!("could not write {}: {e}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff_parse::parse;
    use crate::domain::file_change::{ChangeStatus, FileChange};
    use crate::infrastructure::git_tree::GitWorkingTree;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        jev_testkit::scratch_dir::scratch_dir("diffcmd", name)
    }

    use jev_testkit::git_repo::git;

    fn summary(before: &str, after: &str, file: &str) -> ChangeSummary {
        let f = FileChange { path: file.into(), status: ChangeStatus::Modified, added: 2, removed: 1, old_path: None };
        ChangeSummary { files: vec![f], before: before.into(), after: after.into(), ..ChangeSummary::default() }
    }

    #[test]
    fn merging_spans_the_first_start_to_the_last_end_over_every_file() {
        let m = merge(&[summary("t0", "t1", "a.rs"), summary("t1", "t2", "b.rs"), summary("t2", "t3", "a.rs")]).unwrap();
        assert_eq!((m.before.as_str(), m.after.as_str()), ("t0", "t3"));
        assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.rs", "b.rs"]);
        assert!(merge(&[]).is_none());
    }

    #[test]
    fn scopes_name_a_summary_or_explain_why_not() {
        let h = [summary("t0", "t1", "a"), summary("t1", "t2", "b")];
        assert_eq!(summary_for(Scope::Turn(2), None, &h).unwrap().after, "t2");
        assert!(summary_for(Scope::Turn(3), None, &h).unwrap_err().contains("there is no turn 3"));
        assert!(summary_for(Scope::Last, None, &h).is_err());
        assert_eq!(summary_for(Scope::All, None, &h).unwrap().before, "t0");
        assert!(summary_for(Scope::All, None, &[]).is_err());
    }

    #[test]
    fn reverting_one_hunk_leaves_the_others() {
        let dir = temp("revert");
        git(&dir, &["init", "-q"]);
        let original: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("f.txt"), &original).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        let edited = original.replace("line 2\n", "line TWO\n").replace("line 28\n", "line TWENTY-EIGHT\n");
        std::fs::write(dir.join("f.txt"), &edited).unwrap();
        let files = parse(&git(&dir, &["diff", "--no-color"]));
        assert_eq!(files[0].hunks.len(), 2);

        revert_hunk(&GitWorkingTree, &dir, &files[0], 0).unwrap();
        let now = std::fs::read_to_string(dir.join("f.txt")).unwrap();
        assert!(now.contains("line 2\n") && now.contains("line TWENTY-EIGHT"), "{now}");

        // The hunk no longer matches: refused, file untouched.
        let e = revert_hunk(&GitWorkingTree, &dir, &files[0], 0).unwrap_err();
        assert!(e.contains("f.txt changed since"), "{e}");
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), now);
        assert!(revert_hunk(&GitWorkingTree, &dir, &files[0], 5).unwrap_err().contains("no hunk 6"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_a_patch_git_can_apply_and_never_overwrites() {
        let dir = temp("export");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        std::fs::write(dir.join("f.txt"), "a\nB\n").unwrap();
        let files = parse(&git(&dir, &["diff", "--no-color"]));
        let target = export_patch(&dir, &files, "out.patch").unwrap();
        git(&dir, &["checkout", "--", "f.txt"]);
        git(&dir, &["apply", target.to_str().unwrap()]);
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "a\nB\n");
        assert!(export_patch(&dir, &files, "out.patch").unwrap_err().contains("already exists"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
