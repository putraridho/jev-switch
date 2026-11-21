use super::*;

#[test]
fn file_rules_use_the_absolute_path_spelling_and_the_right_field() {
    let rule = |tool: &str, input: Value, blocked: Option<&str>| allow_rule(tool, &tool_input_from_json(&input), blocked, &places());
    // Every file-editing tool is matched against `Edit(...)` rules only.
    assert_eq!(rule("Write", json!({"file_path": "/work/app/src/a.rs"}), None).as_deref(), Some("Edit(//work/app/src/a.rs)"));
    assert_eq!(rule("Edit", json!({"file_path": "/work/app/a.rs"}), None).as_deref(), Some("Edit(//work/app/a.rs)"));
    assert_eq!(rule("MultiEdit", json!({"file_path": "/work/app/a.rs"}), None).as_deref(), Some("Edit(//work/app/a.rs)"));
    assert_eq!(rule("NotebookEdit", json!({"notebook_path": "/work/app/n.ipynb"}), None).as_deref(), Some("Edit(//work/app/n.ipynb)"));
    // A relative path is the project's, not `/rel`.
    assert_eq!(rule("Write", json!({"file_path": "src/b.rs"}), None).as_deref(), Some("Edit(//work/app/src/b.rs)"));
    assert_eq!(rule("Edit", json!({"file_path": "./b.rs"}), None).as_deref(), Some("Edit(//work/app/b.rs)"));
    assert_eq!(rule("NotebookEdit", json!({"file_path": "/work/app/other"}), None), None, "a notebook is named by notebook_path");
    // A read outside the project allows its folder; a search names the folder itself.
    assert_eq!(rule("Read", json!({"file_path": "/home/u/notes/todo.md"}), None).as_deref(), Some("Read(//home/u/notes/**)"));
    assert_eq!(rule("Grep", json!({"pattern": "x", "path": "/tmp/build/logs"}), None).as_deref(), Some("Read(//tmp/build/logs/**)"));
    assert_eq!(rule("Read", json!({"file_path": "/work/app/a.rs"}), None).as_deref(), Some("Read(//work/app/a.rs)"));
    // The blocked path Claude Code reports wins over the input.
    assert_eq!(rule("Read", json!({"file_path": "x"}), Some("/home/u/data/f.txt")).as_deref(), Some("Read(//home/u/data/**)"));
}

#[test]
fn a_write_rule_is_only_for_a_plain_project_file_or_a_grantable_folder() {
    let rule = |tool: &str, path: &str| rule_for(tool, &tool_input_from_json(&json!({"file_path": path})), None, &places());
    for (path, want) in [
        // Settings, hooks, env files, credentials: writing them for good could grant or run anything.
        ("/work/app/.claude/settings.local.json", Refusal::SensitiveFile),
        (".claude/settings.json", Refusal::SensitiveFile),
        ("/work/app/.git/hooks/pre-commit", Refusal::SensitiveFile),
        ("/work/app/.envrc", Refusal::SensitiveFile),
        ("/work/app/.env", Refusal::SensitiveFile),
        ("/work/app/config/secrets/key.pem", Refusal::SensitiveFile),
        ("/home/u/notes/.hidden", Refusal::SensitiveFile),
        // Outside the project only a grantable folder qualifies.
        ("/home/u/.bashrc", Refusal::SensitiveFile),
        ("/home/u/.ssh/authorized_keys", Refusal::FolderTooBroad),
        ("/home/u/notes.txt", Refusal::FolderTooBroad),
        ("/etc/hosts", Refusal::FolderTooBroad),
        ("/usr/local/bin/x", Refusal::FolderTooBroad),
        ("/tmp/x.txt", Refusal::FolderTooBroad),
        ("/work/other/a.rs", Refusal::FolderTooBroad),
        ("/work/app", Refusal::SensitiveFile),
    ] {
        for tool in ["Write", "Edit"] {
            assert_eq!(rule(tool, path), Err(want), "{tool} {path}");
        }
    }
    assert_eq!(rule("Write", "/home/u/notes/todo.md").as_deref(), Ok("Edit(//home/u/notes/todo.md)"));
    assert_eq!(rule("Write", "/tmp/build/out.txt").as_deref(), Ok("Edit(//tmp/build/out.txt)"));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_cannot_lead_a_write_rule_out_of_the_project() {
    let dir = tmp("write-link");
    let project = dir.join("app");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::os::unix::fs::symlink("/etc", project.join("etc")).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", project.join("hosts")).unwrap();
    let places = Places { project: project.clone(), home: Some(PathBuf::from("/home/u")), temp: PathBuf::from("/nonexistent-temp") };
    let rule = |path: &Path| rule_for("Write", &tool_input_from_json(&json!({"file_path": path.to_str().unwrap()})), None, &places);
    assert_eq!(rule(&project.join("etc/hosts")), Err(Refusal::FolderTooBroad), "a folder link out");
    assert_eq!(rule(&project.join("hosts")), Err(Refusal::UnsafePath), "a file link");
    assert!(rule(&project.join("src/a.rs")).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_path_that_could_match_more_than_it_names_gets_no_rule() {
    let rule = |tool: &str, path: &str| allow_rule(tool, &tool_input_from_json(&json!({"file_path": path})), None, &places());
    for path in ["/work/app/*", "/work/app/**/x", "/work/app/a?.rs", "/work/app/[ab].rs", "/work/../etc/passwd", "/w/a)b", "/w/(a", "/w/{a,b}", "/w/a\nb", ""] {
        for tool in ["Write", "Edit", "Read"] {
            assert_eq!(rule(tool, path), None, "{tool} {path:?}");
        }
    }
}

#[test]
fn web_and_mcp_rules() {
    let web = |u: &str| allow_rule("WebFetch", &tool_input_from_json(&json!({"url": u})), None, &places());
    assert_eq!(web("https://docs.rs/serde/latest?x=1").as_deref(), Some("WebFetch(domain:docs.rs)"));
    assert_eq!(web("http://user@localhost:8080/a").as_deref(), Some("WebFetch(domain:localhost)"));
    assert_eq!(web("https:///").as_deref(), None);
    for odd in ["https://*.evil.com/x", "https://[::1]/x", "https://a b.com", "https://.x.com/", "https://x.com)/"] {
        assert_eq!(web(odd), None, "{odd}");
    }
    assert_eq!(allow_rule("mcp__jev__delegate", &tool_input_from_json(&json!({})), None, &places()).as_deref(), Some("mcp__jev__delegate"));
    assert_eq!(allow_rule("mcp__a(b)", &tool_input_from_json(&json!({})), None, &places()), None);
    assert_eq!(allow_rule("WebSearch", &tool_input_from_json(&json!({"query": "x"})), None, &places()).as_deref(), Some("WebSearch"));
}

#[test]
fn git_failures_do_not_trust_the_local_settings() {
    assert!(tracked_from_git(Some(0), ""));
    assert!(!tracked_from_git(Some(1), "error: pathspec did not match"));
    assert!(!tracked_from_git(Some(128), "fatal: not a git repository (or any of the parent directories): .git"));
    assert!(tracked_from_git(Some(128), "fatal: detected dubious ownership in repository at '/x'"));
    assert!(tracked_from_git(Some(128), "fatal: index file corrupt"));
    assert!(tracked_from_git(Some(128), "fatal: unsafe repository ('/x' is owned by someone else)"));
    // A passing failure does not switch the person's own rules off.
    assert!(!tracked_from_git(Some(128), "fatal: Unable to create '/x/.git/index.lock': File exists."));
    assert!(!tracked_from_git(Some(128), "fatal: cannot change to '/gone': No such file or directory"));
    assert!(tracked_from_git(None, ""), "git killed by a signal says nothing: not the person's own");
}
