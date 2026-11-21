//! The session commands beyond the core loop: context (`/compact`, `/context`, `/cost`,
//! `/export`, `/copy`), composing (`/edit`, `!command`, attachments), the project
//! (`/init`, `/memory`, `/review`) and setup (`/config`, `/terminal-setup`). Wording comes
//! from [`crate::render`]; the programs they start come from [`crate::app::host`].


use std::path::{Path, PathBuf};

mod compose;
mod context;
mod setup;


/// `/init`: what the agent is asked to do.
pub(super) const INIT_PROMPT: &str = "Write the project instructions file for coding agents. Look at this repository first: its \
purpose, layout, languages, how to build, run, lint and test it, the conventions the code follows, and anything a new \
contributor would trip over. Then create AGENTS.md at the project root (or update it if it exists, keeping what is still \
true), short and concrete: commands in code blocks, no generic advice. Different coding CLIs read different files, so \
make each one point at AGENTS.md: if CLAUDE.md does not exist, create it with the single line `@AGENTS.md`; if GEMINI.md \
does not exist, create it with the single line `@AGENTS.md`. Do not change any existing CLAUDE.md or GEMINI.md beyond \
adding that line when it is missing. Finish with a two-line summary of what you wrote.";

/// `/review`: what the agent is asked to do, read-only.
pub(super) fn review_prompt(focus: Option<&str>) -> String {
    let mut prompt = "Review the uncommitted changes in this repository (run `git status` and `git diff HEAD`, and read \
the surrounding code where needed). Look for real problems: bugs, wrong edge cases, broken error handling, security \
issues, races, and missing tests for new behaviour. For each finding give the file and line, what goes wrong with a \
concrete input, and the fix. Skip style nits. Do not edit any file. If there are no changes, say so."
        .to_string();
    if let Some(f) = focus {
        prompt.push_str(&format!(" Focus on: {f}."));
    }
    prompt
}


/// A path the person typed: `~/` from home, relative ones from the project.
pub(super) fn resolve(typed: &str, project: &Path, home: Option<&str>) -> PathBuf {
    match (typed.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => Path::new(h).join(rest),
        _ if Path::new(typed).is_absolute() => PathBuf::from(typed),
        _ => project.join(typed),
    }
}

/// `path` with the home folder written as `~`.
fn tilde(path: &Path, home: Option<&str>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `text` cut to `max` characters, with `…` when cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_resolve_from_home_or_the_project() {
        let project = Path::new("/p");
        assert_eq!(resolve("~/a.md", project, Some("/home/u")), PathBuf::from("/home/u/a.md"));
        assert_eq!(resolve("/tmp/a.md", project, Some("/home/u")), PathBuf::from("/tmp/a.md"));
        assert_eq!(resolve("notes/a.md", project, None), PathBuf::from("/p/notes/a.md"));
        assert_eq!(tilde(Path::new("/home/u/.config/x"), Some("/home/u")), "~/.config/x");
        assert_eq!(tilde(Path::new("/etc/x"), Some("/home/u")), "/etc/x");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
    }
}
