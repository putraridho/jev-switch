use super::git_command;
#[cfg(unix)]
use super::git_line;

#[test]
fn status_bar_git_commands_disable_repo_programs() {
    let dir = std::path::Path::new(".");
    for args in [&["rev-parse", "--abbrev-ref", "HEAD"][..], &["status", "--porcelain"][..]] {
        let command = git_command(dir, args);
        let actual: Vec<_> = command.get_args().collect();
        let hardening = crate::shared::infrastructure::git_guard::no_repo_programs(dir);
        let expected: Vec<_> = hardening.iter().map(String::as_str).chain(args.iter().copied()).map(std::ffi::OsStr::new).collect();
        assert_eq!(actual, expected);
    }
}

#[cfg(unix)]
#[test]
fn status_bar_does_not_run_repo_fsmonitor() {
    use std::os::unix::fs::PermissionsExt;

    let repo = crate::shared::infrastructure::git_guard::scratch::Repo::new("fsmonitor");
    repo.git(&["commit", "--allow-empty", "-qm", "init"]);
    let (script, marker) = (repo.path.join(".git/fsmonitor-test.sh"), repo.marker());
    std::fs::write(&script, format!("#!/bin/sh\nprintf x > {}\n", marker.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    repo.git(&["config", "core.fsmonitor", &script.to_string_lossy()]);

    assert!(git_line(&repo.path).is_some());
    assert!(!marker.exists(), "the status bar must not run the configured fsmonitor");
    assert!(repo.runs::<&str>(&[], &["status", "--porcelain"]));
    assert!(marker.exists(), "without the override the fsmonitor runs (the test is real)");
}
