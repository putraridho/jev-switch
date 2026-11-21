//! What a worker is told before the brief, and where a read-only worker may run.

use jev_backend::ProviderId;

/// What a worker is told before the brief: where it may work and how to report.
pub fn worker_prompt(brief: &str, files: &[String], read_only: bool) -> String {
    let hints = if files.is_empty() {
        String::new()
    } else {
        format!("\n\nFiles that are probably involved (hints, not limits): {}", files.join(", "))
    };
    let rules = if read_only {
        "You are a worker on one READ-ONLY piece of a larger task. Rules: read and investigate inside the current \
         directory only; do not create, change or delete any file, and do not run commands that change the project; do \
         not use the network. When you are done, reply with a report of what you found, with file paths and line \
         numbers where they help."
    } else {
        "You are a worker on one piece of a larger task. Rules: work only inside the current directory (your own git \
         worktree) and do not read or write outside it; do not use the network; do not commit or touch git branches. \
         Run the project's tests before you finish: your work is merged only if they pass. When you are done, reply \
         with a short report: what you changed, and anything you could not do."
    };
    format!("{rules}\n\nTask:\n{brief}{hints}")
}

/// Whether the provider's CLI is held to plan mode by the CLI itself, so a read-only worker
/// can run in the user's own folder. Claude Code (and DeepSeek, which runs on it) has a real
/// plan mode and Codex a read-only sandbox; Gemini and Grok only fall back to their default
/// approval mode, which is not a guarantee, so their read-only workers get a worktree.
pub fn plan_mode_enforced(provider: ProviderId) -> bool {
    match provider {
        ProviderId::Claude | ProviderId::Codex | ProviderId::DeepSeek => true,
        ProviderId::Gemini | ProviderId::Grok => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_backend::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

    #[test]
    fn the_worker_prompt_confines_the_worker_and_asks_for_a_short_report() {
        let p = worker_prompt("add a retry", &["src/net.rs".into()], false);
        assert!(p.contains("only inside the current directory") && p.contains("do not use the network"));
        assert!(p.contains("short report") && p.contains("add a retry") && p.contains("src/net.rs"));
        assert!(p.contains("Run the project's tests") && p.contains("merged only if they pass"));
        assert!(!worker_prompt("x", &[], false).contains("Files that are probably involved"));
        // A read-only worker is told to change nothing, and nothing about merging.
        let r = worker_prompt("find the callers", &[], true);
        assert!(r.contains("READ-ONLY") && r.contains("do not create, change or delete any file") && r.contains("find the callers"));
        assert!(!r.contains("merged") && !r.contains("worktree"));
    }

    #[test]
    fn only_clis_with_a_real_plan_mode_read_in_place() {
        let in_place: Vec<ProviderId> = PROVIDERS.into_iter().filter(|p| plan_mode_enforced(*p)).collect();
        assert_eq!(in_place, [Claude, DeepSeek, Codex]);
    }
}
