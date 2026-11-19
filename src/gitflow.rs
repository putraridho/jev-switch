//! `/commit` and `/pr`: the agent is asked to do the git work, with a prompt that says what
//! a good commit and a good pull request are in this repository. The prompts are pure, so
//! what they promise (never push without being told, never commit secrets) is a unit test.

use std::path::Path;
use std::process::{Command, Stdio};

/// Put before the subcommand of every git call jev-code makes by itself: a repository's
/// `core.fsmonitor` names a program git runs on `status`/`diff`, and an agent that may
/// edit files could set it in `.git/config`. An empty value turns it off in every git version.
pub const NO_REPO_PROGRAMS: [&str; 2] = ["-c", "core.fsmonitor="];

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
}
