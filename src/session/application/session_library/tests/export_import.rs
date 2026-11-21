use super::transfer::*;
use super::*;

#[test]
fn an_export_imports_into_another_project_and_resumes_there() {
    let (state, a, _) = library("export");
    let entry = scan_all(&FileSessions, &state).into_iter().find(|e| e.title() == "fix the lexer").unwrap();
    let file = state.join("out.json");
    export_to(&SystemHost, &entry, &file).unwrap();
    assert!(export_to(&SystemHost, &entry, &file).is_err(), "never overwrites");
    let other = state.join("proj-c");
    std::fs::create_dir_all(&other).unwrap();
    let moved = import_file(&FileSessions, &SystemHost, &state, &other, &file, NOW).unwrap();
    let listed = session_file::list_sessions(&state, &other, NOW, None);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, moved);
    assert_eq!(listed[0].title, "fix the lexer");
    // The original stays where it was.
    // (The 20-day-old one is past the 14 days `/resume` offers.)
    assert_eq!(session_file::list_sessions(&state, &a, NOW, None).len(), 1);
    assert!(matches!(import_file(&FileSessions, &SystemHost, &state, &other, &state.join("nope.json"), NOW), Err(ImportError::Unreadable(_))));
    std::fs::write(state.join("junk.json"), "{}").unwrap();
    assert!(matches!(import_file(&FileSessions, &SystemHost, &state, &other, &state.join("junk.json"), NOW), Err(ImportError::NotASession(_))));
}

#[test]
fn export_masks_secrets_without_changing_the_stored_session() {
    let state = temp("export-secrets");
    let project = state.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let original_path = put(&state, &project, 1, vec![turn(
        ProviderId::Claude,
        r#"use {"password": "hunter2hunter2"}"#,
        "SECRET = replysecret",
        &["Bash curl https://alice:toolpassword@example.com", "Read xai-abcdefghijklmnopqrstuvwx"],
    )]);
    let original = std::fs::read_to_string(&original_path).unwrap();
    let entry = scan_all(&FileSessions, &state).into_iter().next().unwrap();
    let output = state.join("export.json");
    export_to(&SystemHost, &entry, &output).unwrap();
    let exported = std::fs::read_to_string(&output).unwrap();
    for secret in ["hunter2", "replysecret", "toolpassword", "xai-abcdefghijklmnopqrstuvwx"] {
        assert!(!exported.contains(secret), "{exported}");
        assert!(original.contains(secret));
    }
    let saved = SavedSession::parse(&exported).unwrap();
    let turn = &saved.sessions.log()[0];
    assert!(turn.prompt.contains("[REDACTED]"));
    assert!(turn.reply.contains("[REDACTED]"));
    assert!(turn.tools.iter().all(|tool| tool.contains("[REDACTED]")));
    assert_eq!(std::fs::read_to_string(original_path).unwrap(), original);
    assert!(entry.saved.sessions.log()[0].prompt.contains("hunter2"));
}
