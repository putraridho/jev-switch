use super::*;

const BLOCKED_FIXTURE: &str = fixture!("claude/permission_allow.jsonl");

/// The recorded request: Claude Code asked about a `touch` because `/tmp/spike/outside.txt`
/// is outside the allowed folders.
fn recorded() -> (Value, String) {
    let ev: Value = BLOCKED_FIXTURE.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["type"] == "control_request").unwrap();
    (ev["request"]["input"].clone(), ev["request"]["blocked_path"].as_str().unwrap().to_string())
}

#[test]
fn a_path_blocked_bash_call_gets_its_folder_and_a_command_rule() {
    let (input, blocked) = recorded();
    let plan = always_plan("Bash", &input, Some(&blocked), &places()).unwrap();
    assert_eq!(plan, Always { rules: vec!["Bash(touch:*)".into()], folder: Some(PathBuf::from("/tmp/spike")) });
    assert_eq!(plan.label(), "Bash(touch:*) and folder /tmp/spike");
}

#[test]
fn without_a_safe_command_rule_the_folder_is_still_remembered_alone() {
    let (_, blocked) = recorded();
    let input = bash("bash -c 'touch /tmp/spike/outside.txt'");
    let plan = always_plan("Bash", &input, Some(&blocked), &places()).unwrap();
    assert_eq!(plan, Always { rules: vec![], folder: Some(PathBuf::from("/tmp/spike")) });
    assert_eq!(plan.label(), "folder /tmp/spike");
}

#[test]
fn a_folder_that_is_a_folder_is_allowed_as_itself_and_a_file_by_its_parent() {
    let dir = tmp("blocked-dir");
    let sub = dir.join("data");
    std::fs::create_dir_all(&sub).unwrap();
    let places = Places { temp: std::env::temp_dir(), ..places() };
    let folder = |p: &Path| blocked_folder(p.to_str().unwrap(), &places);
    assert_eq!(folder(&sub), Ok(sub.clone()));
    assert_eq!(folder(&sub.join("new.txt")), Ok(sub.clone()));
    assert_eq!(folder(&sub.join("sub/deeper/file")), Ok(sub.join("sub/deeper")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_folders_inside_home_or_temp_that_hold_no_secrets_are_granted_for_good() {
    let places = places();
    for (path, want) in [
        ("/", Refusal::FolderTooBroad),
        ("/etc/hosts", Refusal::FolderTooBroad),
        ("/usr/bin/ls", Refusal::FolderTooBroad),
        ("/etc/ssh/sshd_config", Refusal::FolderTooBroad),
        ("/home/other/x/notes.txt", Refusal::FolderTooBroad),
        ("/opt/data/f.txt", Refusal::FolderTooBroad),
        ("/tmp/x.txt", Refusal::FolderTooBroad),
        ("/home/u/notes.txt", Refusal::FolderTooBroad),
        ("/home/notes.txt", Refusal::FolderTooBroad),
        ("/home/u/.ssh/known_hosts", Refusal::FolderTooBroad),
        ("/home/u/.gnupg/x", Refusal::FolderTooBroad),
        ("/home/u/.aws/config", Refusal::FolderTooBroad),
        ("/home/u/.config/x/y", Refusal::FolderTooBroad),
        ("/home/u/.local/share/x", Refusal::FolderTooBroad),
        ("/home/u/.password-store/a.gpg", Refusal::FolderTooBroad),
        ("/home/u/.kube/config", Refusal::FolderTooBroad),
        ("/home/u/.docker/config.json", Refusal::FolderTooBroad),
        ("/home/u/.mozilla/firefox/p/x", Refusal::FolderTooBroad),
        ("/home/u/stuff/keyrings/k", Refusal::FolderTooBroad),
        ("rel/x", Refusal::UnsafePath),
        ("/tmp/a/../../etc/x", Refusal::UnsafePath),
        ("/tmp/a/*/x", Refusal::UnsafePath),
    ] {
        assert_eq!(blocked_folder(path, &places), Err(want), "{path}");
    }
    assert_eq!(blocked_folder("/home/u/project/x.txt", &places), Ok(PathBuf::from("/home/u/project")));
    assert_eq!(blocked_folder("/tmp/spike/x.txt", &places), Ok(PathBuf::from("/tmp/spike")));
    // With the folder refused, `a` is not offered even though the command rule is safe.
    assert_eq!(always_plan("Bash", &bash("touch /etc/hosts"), Some("/etc/hosts"), &places), Err(Refusal::FolderTooBroad));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_inside_home_cannot_lead_a_grant_out_of_it() {
    let dir = tmp("link-out");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink("/etc", home.join("innocent")).unwrap();
    let places = Places { project: project().to_path_buf(), home: Some(home.clone()), temp: PathBuf::from("/nonexistent-temp") };
    assert_eq!(grantable_folder(&home.join("innocent"), &places), Err(Refusal::FolderTooBroad));
    std::fs::create_dir_all(home.join("real")).unwrap();
    assert_eq!(grantable_folder(&home.join("real"), &places), Ok(()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_blocked_tools_keep_their_rules_and_add_no_folder() {
    let plan = always_plan("Read", &json!({"file_path": "x"}), Some("/home/u/data/f.txt"), &places()).unwrap();
    assert_eq!(plan, Always { rules: vec!["Read(//home/u/data/**)".into()], folder: None });
    assert_eq!(always_plan("Task", &json!({}), None, &places()), Err(Refusal::NoRule));
    assert_eq!(always_plan("Bash", &bash("bash -c x"), None, &places()), Err(Refusal::RunsOtherCommands));
}
