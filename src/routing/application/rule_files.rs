//! Reading the user's routing rule files.

use std::path::Path;

use crate::routing::domain::user_rules::{RuleError, Rules};

/// The user's file, then the project's; later rules apply after earlier ones. A file that
/// is missing or unreadable adds nothing.
pub fn load(state_dir: Option<&Path>, project: &Path) -> (Rules, Vec<RuleError>) {
    let texts: Vec<String> = Rules::file_paths(state_dir, project).into_iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect();
    Rules::parse_files(texts.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_load_user_then_project() {
        let base = std::env::temp_dir().join(format!("jev-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (state, project) = (base.join("state"), base.join("proj"));
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(state.join("routing.rules"), "avoid grok\n").unwrap();
        std::fs::write(project.join(".jev/routing.rules"), "prefer codex\nbad\n").unwrap();
        let (r, errors) = load(Some(&state), &project);
        assert_eq!(r.lines().collect::<Vec<_>>(), vec!["avoid grok", "prefer codex"]);
        assert_eq!(errors.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
