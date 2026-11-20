//! Real git worktrees for workers: `git worktree add --detach`, the diff of a worker's
//! changes, and applying or reversing a patch on the main tree without leaving it half
//! changed. The only door to git the delegator has.

use std::io::Write as _;
use std::path::{ Component, Path, PathBuf };
use std::process::{ Command, Stdio };

use anyhow::{ anyhow, bail, Context, Result };

use crate::orchestration::application::ports::Worktrees;
use crate::orchestration::domain::file_patch::patch_paths;

/// Run `git -C dir args`, optionally feeding `stdin`; stdout on success, a message with
/// git's own words on failure.
fn git(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(crate::shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git (is it installed?)")?;
    // A separate thread, so a big patch cannot deadlock against git printing before it has read it all.
    let feeder = stdin.zip(child.stdin.take()).map(|(text, mut pipe)| {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        })
    });
    let out = child.wait_with_output().context("waiting for git")?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {}", args.first().unwrap_or(&""), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Files and the index as they were, so a failed merge can put the tree back exactly.
/// `git apply --3way` writes conflict markers into the working tree and stages partial
/// results when it fails, which is not "leave the tree untouched".
struct Snapshot {
    index: Option<(PathBuf, Vec<u8>)>,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Snapshot {
    fn take(repo: &Path, index: Option<PathBuf>, paths: &[String]) -> Self {
        let safe = |p: &&String|
            Path::new(p)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        let files = paths
            .iter()
            .filter(safe)
            .map(|p| repo.join(p))
            .map(|p| {
                let bytes = std::fs::read(&p).ok();
                (p, bytes)
            });
        Self {
            index: index.and_then(|p|
                std::fs
                    ::read(&p)
                    .ok()
                    .map(|b| (p, b))
            ),
            files: files.collect(),
        }
    }

    fn restore(&self) {
        for (path, bytes) in &self.files {
            match bytes {
                Some(b) => {
                    let _ = std::fs::write(path, b);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        if let Some((path, bytes)) = &self.index {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Why a patch did not go on, from git's words for the plain apply and the 3-way one.
fn explain_apply_failure(plain: &str, three_way: &str) -> String {
    let cause = if three_way.contains("does not match index") {
        "your working tree has uncommitted edits to a file the worker also changed, and they overlap its hunks \
         (a 3-way merge also needs those files unmodified or staged). Commit, stage or stash them, or merge by hand"
    } else if plain.contains("already exists") || three_way.contains("already exists") {
        "a file the worker created already exists in your tree"
    } else {
        "the worker's changes do not apply cleanly: they conflict with your tree"
    };
    format!("{cause}; nothing was changed. Plain apply: {plain} 3-way apply: {three_way}")
}

/// `path` as a canonical location strictly inside `root`, or why it is not one. `path`
/// may be untrusted (it was read from a saved record), so: it must be absolute and free of
/// `..`; its parent is canonicalized (the entry itself may be gone) so a symlinked
/// directory on the way cannot lead outside; the entry may not itself be a symlink; and it
/// may not be `root` itself.
fn checked_inside(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("refusing to touch {}: not an absolute path without `..`", path.display());
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("worktrees root {} is not usable", root.display()))?;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        bail!("refusing to touch {}: it has no parent directory and name", path.display());
    };
    let Ok(parent) = parent.canonicalize() else {
        bail!("refusing to touch {}: its parent directory cannot be resolved", path.display());
    };
    let candidate = parent.join(name);
    if candidate == root || !candidate.starts_with(&root) {
        bail!("refusing to touch {}: not strictly inside {}", path.display(), root.display());
    }
    if std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("refusing to touch {}: it is a symlink", path.display());
    }
    Ok(candidate)
}

/// Real worktrees: `git worktree add --detach <root>/<id> HEAD`. Detached, so no branch
/// is created and nothing is left behind once the worktree is removed.
pub struct GitWorktrees {
    /// The top level of the repository: workers start there, and merges apply there.
    repo: PathBuf,
    root: PathBuf,
}

impl GitWorktrees {
    /// `dir` is any directory inside the repository; worktrees go under `root`
    /// (`<state dir>/worktrees`).
    pub fn new(dir: &Path, root: PathBuf) -> Result<Self> {
        let top = git(dir, &["rev-parse", "--show-toplevel"], None).with_context(||
            format!(
                "{} is not inside a git repository, so workers cannot get worktrees",
                dir.display()
            )
        )?;
        Ok(Self { repo: PathBuf::from(top.trim()), root })
    }

    fn index_path(&self) -> Option<PathBuf> {
        let rel = git(&self.repo, &["rev-parse", "--git-path", "index"], None).ok()?;
        Some(self.repo.join(rel.trim()))
    }

    /// A plain `git apply` first: it works on the working tree whatever is unstaged, as
    /// long as the hunks fit. Only when it does not fit, `--3way`, which also merges into
    /// text the planner changed but needs the index to match the files. On failure the
    /// files and index are put back exactly.
    fn apply_patch(&self, patch: &str, direction: Direction) -> Result<()> {
        let run = |extra: &[&str]| {
            let args: Vec<&str> = ["apply"]
                .into_iter()
                .chain(direction.flag())
                .chain(extra.iter().copied())
                .chain(["-"])
                .collect();
            git(&self.repo, &args, Some(patch))
        };
        let snapshot = Snapshot::take(&self.repo, self.index_path(), &patch_paths(patch));
        let Err(plain) = run(&["--check"]).and_then(|_| run(&[])) else {
            return Ok(());
        };
        match run(&["--3way"]) {
            Ok(_) => Ok(()),
            Err(three_way) => {
                snapshot.restore();
                Err(
                    anyhow!(
                        "{}",
                        explain_apply_failure(&format!("{plain:#}."), &format!("{three_way:#}"))
                    )
                )
            }
        }
    }
}

/// Which way a patch goes onto the main tree: a merge, or the undo of one.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    fn flag(self) -> Option<&'static str> {
        match self {
            Self::Forward => None,
            Self::Reverse => Some("-R"),
        }
    }
}

impl Worktrees for GitWorktrees {
    fn repo(&self) -> PathBuf {
        self.repo.clone()
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn create(&self, id: &str) -> Result<PathBuf> {
        let path = self.root.join(id);
        std::fs
            ::create_dir_all(&self.root)
            .with_context(|| format!("create {}", self.root.display()))?;
        git(&self.repo, &["worktree", "add", "--detach", &path.to_string_lossy(), "HEAD"], None)?;
        Ok(path)
    }

    /// `git add -A` first so files the worker created show up; `--binary` so the patch
    /// applies for binary files too.
    fn diff(&self, path: &Path) -> Result<String> {
        git(path, &["add", "-A"], None)?;
        git(path, &["diff", "--cached", "--binary", "--no-ext-diff", "--no-textconv", "HEAD"], None)
    }

    fn apply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Forward)
    }

    /// The same as a merge, in reverse: a 3-way merge's hunks carry the planner's text as
    /// context, so taking them back out needs the 3-way step too.
    fn unapply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Reverse).context(
            "the merged changes cannot be taken back out cleanly"
        )
    }

    /// `git worktree remove`, and when git will not (the checkout belongs to another
    /// repository, or git's record of it is gone) the directory itself. The path comes from
    /// a saved record, so it is trusted only after [`checked_inside`] proves it is a real
    /// entry strictly inside `root`; nothing else is ever deleted or handed to git.
    fn remove(&self, path: &Path) -> Result<()> {
        let inside = checked_inside(&self.root, path)?;
        match git(&self.repo, &["worktree", "remove", "--force", &path.to_string_lossy()], None) {
            Ok(_) => Ok(()),
            Err(_) if !inside.exists() => Ok(()),
            Err(_) =>
                std::fs
                    ::remove_dir_all(&inside)
                    .with_context(|| format!("remove {}", path.display())),
        }
    }

    fn prune(&self, repo: &Path) -> Result<()> {
        git(repo, &["worktree", "prune"], None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        crate::testkit::scratch_dir::unique_scratch_dir("delegate", tag)
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        git(dir, args, None).unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A repository with one commit holding `f.txt` (five lines).
    fn temp_repo() -> PathBuf {
        let d = temp_dir("repo");
        crate::testkit::git_repo::init_with_commit(&d, &[("f.txt", "a\nb\nc\nd\ne\n")]);
        d
    }

    #[test]
    fn real_worktrees_are_isolated_and_diff_includes_new_files() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let (a, b) = (wt.create("wa").unwrap(), wt.create("wb").unwrap());
        assert_ne!(a, b);
        std::fs::write(a.join("f.txt"), "a\nB-from-a\nc\nd\ne\n").unwrap();
        std::fs::write(b.join("new.txt"), "hello\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(b.join("f.txt")).unwrap(),
            "a\nb\nc\nd\ne\n",
            "b does not see a's edit"
        );
        assert!(!a.join("new.txt").exists(), "a does not see b's file");
        assert_eq!(
            std::fs::read_to_string(repo.join("f.txt")).unwrap(),
            "a\nb\nc\nd\ne\n",
            "the main tree is untouched"
        );
        let (da, db) = (wt.diff(&a).unwrap(), wt.diff(&b).unwrap());
        assert_eq!(patch_paths(&da), ["f.txt"]);
        assert_eq!(patch_paths(&db), ["new.txt"]);
        wt.remove(&a).unwrap();
        assert!(!a.exists() && b.exists());
    }

    #[test]
    fn real_merge_applies_a_workers_diff_to_the_main_tree() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        // The planner has since changed another part of the same file: a 3-way merge keeps both.
        std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        wt.apply(&wt.diff(&w).unwrap()).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("f.txt")).unwrap(),
            "a\nB-from-worker\nc\nd\nE-from-planner\n"
        );
        assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn real_undo_takes_a_merge_back_out_plain_or_three_way() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        // A plain merge: the working tree changes, the index does not.
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        wt.apply(&patch).unwrap();
        wt.unapply(&patch).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n");
        assert!(!repo.join("new.txt").exists(), "a file the worker created is gone again");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), "", "the tree is clean again");
        // A 3-way merge (the planner staged an edit to the same file) stages the result; undo puts the index back too.
        std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        let w2 = wt.create("w2").unwrap();
        std::fs::write(w2.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        let patch = wt.diff(&w2).unwrap();
        wt.apply(&patch).unwrap();
        wt.unapply(&patch).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("f.txt")).unwrap(),
            "a\nb\nc\nd\nE-from-planner\n",
            "the planner's edit stays"
        );
        assert_eq!(
            run_git(&repo, &["status", "--porcelain"]),
            "M  f.txt\n",
            "staged as the planner left it, nothing unstaged"
        );
        // After the files moved on, undo refuses and changes nothing.
        wt.apply(&patch).unwrap();
        std::fs::write(repo.join("f.txt"), "rewritten\n").unwrap();
        assert!(format!("{:#}", wt.unapply(&patch).unwrap_err()).contains("nothing was changed"));
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "rewritten\n");
    }

    #[test]
    fn a_conflicting_merge_leaves_the_tree_untouched() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        // The planner changed the very same line.
        std::fs::write(repo.join("f.txt"), "a\nB-from-planner\nc\nd\ne\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        let before_status = run_git(&repo, &["status", "--porcelain"]);
        let before_index = run_git(&repo, &["ls-files", "--stage"]);
        let err = wt.apply(&patch).unwrap_err();
        assert!(format!("{err:#}").contains("nothing was changed"), "{err:#}");
        assert_eq!(
            std::fs::read_to_string(repo.join("f.txt")).unwrap(),
            "a\nB-from-planner\nc\nd\ne\n",
            "no conflict markers"
        );
        assert!(!repo.join("new.txt").exists(), "no partial application");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), before_status);
        assert_eq!(
            run_git(&repo, &["ls-files", "--stage"]),
            before_index,
            "the index is as it was"
        );
    }

    #[test]
    fn worktrees_need_a_git_repository() {
        let not_a_repo = temp_dir("plain");
        let err = GitWorktrees::new(&not_a_repo, temp_dir("root")).err().expect("not a repository");
        assert!(format!("{err:#}").contains("not inside a git repository"));
    }

    /// A repository with `big.txt` of 30 numbered lines, committed.
    fn numbered_repo() -> PathBuf {
        let d = temp_dir("repo");
        crate::testkit::git_repo::init_with_commit(
            &d,
            &[("big.txt", &(1..=30).map(|n| format!("line{n}\n")).collect::<String>())]
        );
        d
    }

    fn replace_line(path: &Path, n: usize, text: &str) {
        let content = std::fs::read_to_string(path).unwrap();
        let out: String = content
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{}\n", if i + 1 == n { text } else { l }))
            .collect();
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn merge_works_on_top_of_the_planners_unstaged_edit_to_the_same_file() {
        let repo = numbered_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        replace_line(&w.join("big.txt"), 3, "from-worker");
        let patch = wt.diff(&w).unwrap();
        // The planner edits another part of the same file and has NOT staged it.
        replace_line(&repo.join("big.txt"), 27, "from-planner");
        wt.apply(&patch).unwrap();
        let merged = std::fs::read_to_string(repo.join("big.txt")).unwrap();
        assert!(merged.contains("from-worker") && merged.contains("from-planner"), "{merged}");
        assert!(
            !merged.contains("line3\n") &&
                !merged.contains("line27\n") &&
                !merged.contains("<<<<<<<")
        );
        assert_eq!(
            run_git(&repo, &["diff", "--cached", "--stat"]).trim(),
            "",
            "the planner's index is not touched"
        );
    }

    #[test]
    fn an_overlapping_unstaged_edit_fails_cleanly_and_says_why() {
        let repo = numbered_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        replace_line(&w.join("big.txt"), 3, "from-worker");
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        replace_line(&repo.join("big.txt"), 4, "from-planner"); // inside the worker's hunk, unstaged
        let (status, before) = (
            run_git(&repo, &["status", "--porcelain"]),
            std::fs::read_to_string(repo.join("big.txt")).unwrap(),
        );
        let err = format!("{:#}", wt.apply(&patch).unwrap_err());
        assert!(err.contains("uncommitted edits") && err.contains("nothing was changed"), "{err}");
        assert!(
            !err.contains("conflict with your tree"),
            "the cause is the dirty file, not a conflict: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("big.txt")).unwrap(),
            before,
            "the file is exactly as it was"
        );
        assert!(!repo.join("new.txt").exists(), "no partial application");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), status);
    }

    #[test]
    fn a_real_checkout_that_is_gone_is_cleaned_by_remove_and_prune() {
        let repo = temp_repo();
        let root = temp_dir("wt-root");
        let wt = GitWorktrees::new(&repo, root.clone()).unwrap();
        let (gone, kept) = (wt.create("wgone").unwrap(), wt.create("wkept").unwrap());
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(
            run_git(&repo, &["worktree", "list"]).contains("wgone"),
            "git still lists the vanished checkout"
        );
        wt.prune(&repo).unwrap();
        let list = run_git(&repo, &["worktree", "list"]);
        assert!(!list.contains("wgone") && list.contains("wkept"), "{list}");
        // A directory git does not know (a leftover of another repository) is deleted, but only under the root.
        let stray = root.join("wstray");
        std::fs::create_dir_all(&stray).unwrap();
        wt.remove(&stray).unwrap();
        assert!(!stray.exists());
        let outside = temp_dir("outside");
        assert!(wt.remove(&outside).is_err() && outside.exists(), "never deletes outside its root");
        wt.remove(&kept).unwrap();
    }

}
