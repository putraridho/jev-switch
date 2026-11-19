//! `/commit` and `/pr`: the agent is asked to do the git work, with a prompt that says what
//! a good commit and a good pull request are in this repository. The prompts are pure, so
//! what they promise (never push without being told, never commit secrets) is a unit test.

use std::path::Path;
use std::process::{Command, Stdio};

/// Put before the subcommand of every git call jev-code makes by itself. It turns off the
/// programs a repository's config names that git would run on its own during the calls jev
/// makes: `core.fsmonitor` (on `status`/`diff`; empty turns it off in every git version) and
/// hooks (`post-checkout` on `worktree add`, `post-merge`; a hooks path with no hooks in it).
/// Our diffs also pass `--no-textconv --no-ext-diff`.
///
/// This is a narrow guard, not a sandbox: filter, merge and credential programs still run,
/// because real repositories depend on them (git-crypt and git-lfs install filter drivers in
/// `.git/config`, and undo and worktrees need them to write real file contents). An agent
/// that can rewrite `.git/config` can usually run commands directly anyway.
pub const NO_REPO_PROGRAMS: [&str; 4] = ["-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null"];

/// The branches a pull request is never opened from.
const TRUNKS: [&str; 3] = ["main", "master", "trunk"];

/// The checked-out branch of `dir`; `None` outside a repository or on a detached head.
pub fn current_branch(dir: &Path) -> Option<String> {
    let out = Command::new("git").args(NO_REPO_PROGRAMS).args(["symbolic-ref", "--short", "-q", "HEAD"]).current_dir(dir).stderr(Stdio::null()).output().ok()?;
    let name = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !name.is_empty()).then_some(name)
}

pub fn is_trunk(branch: &str) -> bool {
    TRUNKS.contains(&branch)
}

/// A branch name made of a task's words: `jev/fix-the-parser`. Lowercase ASCII words joined
/// by dashes, at most 40 characters, never ending in a dash; `jev/work` for a task with no
/// usable words.
pub fn branch_slug(task: &str) -> String {
    let mut slug = String::new();
    for word in task.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let next = if slug.is_empty() { word.to_ascii_lowercase() } else { format!("{slug}-{}", word.to_ascii_lowercase()) };
        if next.len() > 40 {
            break;
        }
        slug = next;
    }
    if slug.is_empty() {
        slug = "work".to_string();
    }
    format!("jev/{slug}")
}

/// `/commit [message hint]`.
pub fn commit_prompt(hint: Option<&str>) -> String {
    let mut prompt = "Commit the current changes in this repository. Run `git status` and `git diff HEAD` to see them, and \
`git log -n 10 --oneline` to learn how this repository words its commit messages. Stage files by name (never `git add -A` \
blindly) and leave out anything that looks like a secret, a key, a `.env` file or a build artifact; say so if you left \
something out. If the changes are about more than one thing, make one commit per thing. Write each message in the \
repository's own style: a short subject saying what changed, and a body saying why when it is not obvious. Do not push, \
do not amend or rewrite earlier commits, and do not skip hooks. Finish by showing `git log -n 3 --oneline`."
        .to_string();
    if let Some(h) = hint {
        prompt.push_str(&format!(" The person says about the change: {h}."));
    }
    prompt
}

/// `/pr [base]`: `branch` is where the repository is now, `task` the person's latest request.
pub fn pr_prompt(base: Option<&str>, branch: Option<&str>, task: Option<&str>) -> String {
    let base = base.unwrap_or("the default branch (main or master)");
    let mut prompt = format!(
        "Prepare a pull request against {base}. Run `git status`, `git log {base}..HEAD --oneline` and `git diff {base}...HEAD` \
to see what the branch changes (use `git merge-base` if the range fails). "
    );
    match branch {
        Some(b) if is_trunk(b) => {
            let name = task.map_or_else(|| "jev/work".to_string(), branch_slug);
            prompt.push_str(&format!("The repository is on `{b}`, which a pull request cannot come from: create the branch `{name}` first (`git switch -c {name}`), carrying the uncommitted changes onto it, and commit them there. "));
        }
        Some(_) => prompt.push_str("Commit any uncommitted work on this branch first, in the repository's commit style. "),
        None => prompt.push_str("The repository is on a detached head: create a branch for it first. "),
    }
    prompt.push_str(
        "Then write the pull request: a title under 70 characters, and a body with a short summary of what changed and why, \
how it was tested, and anything a reviewer should look at first. Never put secrets or local paths in it. If the `gh` \
command is available and signed in, push the branch and open the pull request with `gh pr create`, and print its URL. \
If it is not, do not push: print the title and body so the person can open it themselves.",
    );
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_short_lowercase_words_with_a_fallback() {
        assert_eq!(branch_slug("Fix the Parser, please!"), "jev/fix-the-parser-please");
        assert_eq!(branch_slug("  ✓✓ "), "jev/work");
        assert_eq!(branch_slug(""), "jev/work");
        let long = branch_slug(&"word ".repeat(30));
        assert!(long.len() <= "jev/".len() + 40 && !long.ends_with('-'), "{long}");
        assert_eq!(branch_slug("über cool"), "jev/ber-cool", "non-ASCII letters separate words");
    }

    #[test]
    fn the_commit_prompt_never_pushes_or_rewrites_and_carries_the_hint() {
        let p = commit_prompt(Some("the parser fix"));
        assert!(p.contains("Do not push") && p.contains("do not amend") && p.contains("secret"), "{p}");
        assert!(p.ends_with("the parser fix."), "{p}");
        assert!(!commit_prompt(None).contains("The person says"));
    }

    #[test]
    fn the_pr_prompt_branches_off_a_trunk_and_pushes_only_through_gh() {
        let on_main = pr_prompt(None, Some("main"), Some("Fix the parser"));
        assert!(on_main.contains("`git switch -c jev/fix-the-parser`"), "{on_main}");
        let on_feature = pr_prompt(Some("develop"), Some("feat/x"), None);
        assert!(on_feature.contains("against develop") && !on_feature.contains("git switch -c"), "{on_feature}");
        assert!(pr_prompt(None, None, None).contains("detached head"));
        for p in [&on_main, &on_feature] {
            assert!(p.contains("`gh pr create`") && p.contains("do not push"), "{p}");
        }
    }

    #[test]
    fn the_branch_of_a_real_repository_and_of_a_plain_folder() {
        let dir = std::env::temp_dir().join(format!("jev-gitflow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(current_branch(&dir), None, "not a repository");
        let ok = Command::new("git").args(["init", "-q", "-b", "trunk"]).current_dir(&dir).status().is_ok_and(|s| s.success());
        if ok {
            assert_eq!(current_branch(&dir).as_deref(), Some("trunk"));
            assert!(is_trunk("trunk") && !is_trunk("feat/x"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn hooks_do_not_run_but_the_repos_own_filter_drivers_still_do() {
        let repo = scratch::Repo::new("hooks");
        repo.git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let marker = repo.marker();

        // A post-checkout hook does not run on `worktree add`.
        let hook = repo.path.join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\nprintf x > {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (ours_wt, plain_wt) = (repo.path.join("wt-ours"), repo.path.join("wt-plain"));
        assert!(repo.runs(&NO_REPO_PROGRAMS, &["worktree", "add", "-q", "--detach", &ours_wt.to_string_lossy()]));
        assert!(!marker.exists(), "jev-code's git must not run the repo's hooks");
        assert!(repo.runs(&[], &["worktree", "add", "-q", "--detach", &plain_wt.to_string_lossy()]));
        assert!(marker.exists(), "without the override the hook runs (the test is real)");
        std::fs::remove_file(&marker).unwrap();

        // A filter driver in `.git/config` keeps working: git-crypt and `git lfs install
        // --local` put theirs there, and undo must write real contents, not ciphertext.
        repo.git(&["config", "filter.crypt.clean", &format!("sh -c 'printf x > {}; cat'", marker.display())]);
        std::fs::write(repo.path.join(".gitattributes"), "*.txt filter=crypt\n").unwrap();
        std::fs::write(repo.path.join("a.txt"), "secret\n").unwrap();
        assert!(repo.runs(&NO_REPO_PROGRAMS, &["add", "a.txt"]));
        assert!(marker.exists(), "the repository's own filter driver must still run");
    }
}

/// A throwaway git repository for tests, removed when dropped. Its git calls drop the
/// `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE` a test run can inherit (from a git hook),
/// so they never touch another repository.
#[cfg(test)]
pub(crate) mod scratch {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) struct Repo {
        pub path: PathBuf,
    }

    impl Repo {
        pub fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("jev-scratch-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            let repo = Self { path };
            repo.git(&["init", "-q"]);
            repo
        }

        fn command(&self, prefix: &[&str], args: &[&str]) -> Command {
            let mut command = Command::new("git");
            command
                .args(prefix)
                .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
                .args(args)
                .current_dir(&self.path)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE");
            command
        }

        /// Set the repository up: `git args`, panicking on failure.
        pub fn git(&self, args: &[&str]) -> String {
            let out = self.command(&[], args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        }

        /// `git prefix... args...`; whether it succeeded.
        pub fn runs(&self, prefix: &[&str], args: &[&str]) -> bool {
            self.command(prefix, args).output().is_ok_and(|o| o.status.success())
        }

        /// The file the test's programs write when they run (outside the work tree, so it
        /// never shows up in `git status`).
        pub fn marker(&self) -> PathBuf {
            self.path.join(".git/marker")
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
