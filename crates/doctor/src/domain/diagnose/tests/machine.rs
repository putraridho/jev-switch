use super::*;

#[test]
fn git_and_state_dir_tables() {
    for (value, expect) in [(Some(true), Some(Severity::Ok)), (Some(false), Some(Severity::Warn)), (None, None)] {
        let mut i = inputs(vec![av(Claude, State::Ready)]);
        i.in_git_repo = value;
        i.state_dir_writable = value;
        let r = diagnose(&i);
        assert_eq!(r.checks.iter().find(|c| c.id == "git-repo").map(|c| c.severity), expect);
        assert_eq!(r.checks.iter().find(|c| c.id == "state-dir").map(|c| c.severity), expect);
    }
}

#[test]
fn non_git_folder_warns_with_the_reason_and_fix() {
    let mut i = inputs(vec![]);
    i.in_git_repo = Some(false);
    let c = diagnose(&i).checks.into_iter().find(|c| c.id == "git-repo").unwrap();
    assert!(c.detail.contains("Codex and Gemini") && c.detail.contains("/undo"));
    assert_eq!(c.fix[0], "git init");
}

#[test]
fn env_file_permission_table() {
    // (mode, has_keys, expected severity or None when the check is left out)
    let table = [
        (Some(0o600), true, Some(Severity::Ok)),
        (Some(0o400), true, Some(Severity::Ok)),
        (Some(0o640), true, Some(Severity::Warn)),
        (Some(0o604), true, Some(Severity::Warn)),
        (Some(0o644), true, Some(Severity::Warn)),
        (Some(0o644), false, None),
        (None, true, Some(Severity::Ok)),
    ];
    for (mode, has_keys, expect) in table {
        let mut i = inputs(vec![]);
        i.env_file = Some(EnvFileFacts { path: "/h/.jev-router.env".into(), mode_bits: mode, has_keys });
        let r = diagnose(&i);
        assert_eq!(r.checks.iter().find(|c| c.id == "env-perms").map(|c| c.severity), expect, "{mode:?} {has_keys}");
    }
}

#[test]
fn loose_env_file_gets_the_chmod_fix() {
    let mut i = inputs(vec![]);
    i.env_file = Some(EnvFileFacts { path: "/h/.jev-router.env".into(), mode_bits: Some(0o644), has_keys: true });
    let r = diagnose(&i);
    assert_eq!(find(&r, "env-perms").fix, vec!["chmod 600 /h/.jev-router.env".to_string()]);
    assert!(find(&r, "env-perms").detail.contains("644"));
}
