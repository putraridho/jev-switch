use super::*;
use super::super::completion::*;
use crate::tui::render::CompleteCtx;

fn ctx() -> CompleteCtx {
    CompleteCtx {
        ready_providers: vec!["claude".into(), "codex".into()],
        models: vec!["codex:gpt-6.1-sol".into()],
        changed_files: vec!["src/a.rs".into()],
        skills: Vec::new(),
        mcp_servers: Vec::new(),
    }
}

#[test]
fn tab_completes_commands_and_their_arguments() {
    let (start, c) = completions_for("/mode", 5, &ctx());
    assert_eq!(start, 0);
    assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["/mode", "/model"]);

    let (start, c) = completions_for("/model co", 9, &ctx());
    assert_eq!(start, 7, "only the word being completed is replaced");
    assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["codex", "codex:gpt-6.1-sol"]);

    let (_, c) = completions_for("/diff ", 6, &ctx());
    assert_eq!(c.iter().map(|p| p.replacement.as_str()).collect::<Vec<_>>(), ["src/a.rs"]);
}

#[test]
fn at_paths_complete_from_the_folder_and_tracked_files() {
    let dir = temp("atpath");
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    std::fs::write(dir.join("my notes.md"), "").unwrap();
    std::fs::write(dir.join(".env"), "").unwrap();
    let names = |word: &str, tracked: &[String]| path_candidates(word, &dir, tracked).into_iter().map(|p| p.replacement).collect::<Vec<_>>();
    assert_eq!(names("", &[]), ["@my\\ notes.md", "@src/"]);
    assert_eq!(names("s", &[]), ["@src/"]);
    assert_eq!(names("src/", &[]), ["@src/deep/", "@src/main.rs"]);
    assert_eq!(names("src/M", &[]), ["@src/main.rs"], "case is ignored");
    assert_eq!(names(".", &[]), ["@.env"], "hidden only when asked, never .git");
    assert_eq!(names("main", &["src/main.rs".to_string(), "README.md".to_string()]), ["@src/main.rs"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn prose_and_a_cursor_in_the_middle_are_handled() {
    assert!(completions_for("fix the bug", 11, &ctx()).1.is_empty());
    // The cursor sits after "/mo": the rest of the line is not part of the word.
    let (start, c) = completions_for("/model auto", 3, &ctx());
    assert_eq!(start, 0);
    assert!(c.iter().all(|p| p.replacement.starts_with("/mo")));
    // A position past the end (or inside a multi-byte char) never panics.
    let _ = completions_for("é/mod", 99, &ctx());
}
