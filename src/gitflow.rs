//! `/commit` and `/pr`: the agent is asked to do the git work, with a prompt that says what
//! a good commit and a good pull request are in this repository. The prompts are pure, so
//! what they promise (never push without being told, never commit secrets) is a unit test.

use std::path::Path;
use std::process::{Command, Stdio};

/// The programs a repository's own config can name that git runs without being asked:
/// `core.fsmonitor` (on `status`/`diff`) and hooks (`post-checkout` on `worktree add`,
/// `post-merge`). An empty `core.fsmonitor` turns it off in every git version; a hooks path
/// with no hooks in it turns hooks off.
const NO_FIXED_PROGRAMS: [&str; 4] = ["-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null"];

/// Put before the subcommand of every git call jev-code makes by itself: an agent that may
/// edit files could name a program in `.git/config` that git then runs. Besides
/// [`NO_FIXED_PROGRAMS`], every filter driver the repository's own config defines (a
/// `.gitattributes` line switches it on, and `status`, `add` and checkouts run its `clean`,
/// `smudge` or `process` program) is emptied, which git treats as no filter. Drivers in the
/// person's global or system config (git-lfs) are theirs and stay on. Diff drivers
/// (`textconv`, external diff) only run when a diff call asks for them, and ours pass
/// `--no-textconv --no-ext-diff`.
pub fn no_repo_programs(dir: &Path) -> Vec<String> {
    let mut args: Vec<String> = NO_FIXED_PROGRAMS.iter().map(|s| (*s).to_string()).collect();
    for driver in repo_filter_drivers(dir) {
        for (key, value) in [("clean", ""), ("smudge", ""), ("process", ""), ("required", "false")] {
            args.push("-c".to_string());
            args.push(format!("filter.{driver}.{key}={value}"));
        }
    }
    args
}

/// The filter drivers named in the repository's own config files (`.git/config`, its
/// `config.worktree`, and what they include). Reading config runs no program.
fn repo_filter_drivers(dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(NO_FIXED_PROGRAMS)
        .args(["config", "--show-scope", "--includes", "--null", "--get-regexp", r"^filter\..+\.(clean|smudge|process)$"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    out.map_or_else(|_| Vec::new(), |o| repo_driver_names(&String::from_utf8_lossy(&o.stdout)))
}

/// The driver names from the repository's own scopes (`local`, `worktree`) in
/// `git config --show-scope --null --get-regexp` output: `scope\0key\nvalue\0` per entry,
/// keys `filter.<driver>.<variable>` (the driver may itself contain dots).
fn repo_driver_names(listing: &str) -> Vec<String> {
    let mut fields = listing.split('\0');
    let mut drivers: Vec<String> = Vec::new();
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let key = entry.split('\n').next().unwrap_or_default();
        let Some((driver, _)) = key.strip_prefix("filter.").and_then(|rest| rest.rsplit_once('.')) else { continue };
        if !driver.is_empty() && !drivers.iter().any(|d| d == driver) {
            drivers.push(driver.to_string());
        }
    }
    drivers
}

/// The branches a pull request is never opened from.
const TRUNKS: [&str; 3] = ["main", "master", "trunk"];

/// The checked-out branch of `dir`; `None` outside a repository or on a detached head.
pub fn current_branch(dir: &Path) -> Option<String> {
    let out = Command::new("git").args(no_repo_programs(dir)).args(["symbolic-ref", "--short", "-q", "HEAD"]).current_dir(dir).stderr(Stdio::null()).output().ok()?;
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

    #[test]
    fn driver_names_come_from_the_repos_own_scopes_only() {
        let listing = "local\0filter.evil.clean\nsh -c x\0worktree\0filter.a.b.process\ny\0local\0filter.evil.smudge\nz\0\
global\0filter.lfs.clean\ngit-lfs clean\0system\0filter.sys.clean\nq\0local\0diff.t.textconv\nz\0";
        assert_eq!(repo_driver_names(listing), ["evil", "a.b"]);
        assert!(repo_driver_names("").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_repo_filter_driver_and_hooks_do_not_run() {
        let repo = scratch::Repo::new("filter");
        repo.git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let marker = repo.marker();
        repo.git(&["config", "filter.evil.clean", &format!("sh -c 'printf x > {}; cat'", marker.display())]);
        repo.git(&["config", "filter.evil.required", "true"]);
        std::fs::write(repo.path.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(repo.path.join("a.txt"), "two\n").unwrap();

        let ours = no_repo_programs(&repo.path);
        assert!(ours.contains(&"filter.evil.clean=".to_string()), "{ours:?}");
        assert!(repo.runs(&ours, &["add", "a.txt"]), "an emptied filter is no filter, even when required");
        assert!(!marker.exists(), "jev-code's git must not run the repo's filter");
        std::fs::write(repo.path.join("a.txt"), "three\n").unwrap();
        assert!(repo.runs(&[], &["add", "a.txt"]));
        assert!(marker.exists(), "without the override the configured filter runs (the test is real)");
        std::fs::remove_file(&marker).unwrap();

        // A post-checkout hook does not run on `worktree add` either.
        let hook = repo.path.join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\nprintf x > {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (ours_wt, plain_wt) = (repo.path.join("wt-ours"), repo.path.join("wt-plain"));
        assert!(repo.runs(&no_repo_programs(&repo.path), &["worktree", "add", "-q", "--detach", &ours_wt.to_string_lossy()]));
        assert!(!marker.exists(), "jev-code's git must not run the repo's hooks");
        assert!(repo.runs(&[], &["worktree", "add", "-q", "--detach", &plain_wt.to_string_lossy()]));
        assert!(marker.exists(), "without the override the hook runs (the test is real)");
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

        fn command(&self, prefix: &[String], args: &[&str]) -> Command {
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
        pub fn runs(&self, prefix: &[String], args: &[&str]) -> bool {
            self.command(prefix, args).output().is_ok_and(|o| o.status.success())
        }

        /// The file the test's hostile programs write when they run (outside the work tree,
        /// so it never shows up in `git status`).
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
