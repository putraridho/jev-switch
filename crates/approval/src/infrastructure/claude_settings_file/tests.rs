use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::infrastructure::tool_input_wire::tool_input_from_json;
use super::*;
use crate::domain::always_plan::always_plan;
use jev_shared::infrastructure::system_host::SystemHost;

fn project() -> &'static Path {
    Path::new("/work/app")
}

fn places() -> Places {
    Places { project: project().to_path_buf(), home: Some(PathBuf::from("/home/u")), temp: PathBuf::from("/tmp"), disk: crate::domain::grantable::Disk(std::sync::Arc::new(crate::infrastructure::path_facts::SystemPathFacts)) }
}

fn tmp(name: &str) -> PathBuf {
    jev_testkit::scratch_dir::scratch_dir("perm", name)
}

fn bash(command: &str) -> Value {
    json!({ "command": command })
}

/// Every edit kind the module makes to `.claude/settings.local.json`, in order, each with
/// the file as it stands afterwards. `before` is what the file holds to begin with.
fn edit_steps(name: &str, before: Option<&str>, rule: &str, dir: &str) -> String {
    use super::{add_allow_rule, add_directory, local_settings_path, remove_allow_rule, save_always, Always};
    let project = tmp(name);
    let path = local_settings_path(&project);
    if let Some(text) = before {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    let always = Always { rules: vec!["Bash(grep:*)".into(), "Edit".into()], folder: Some(PathBuf::from("/work/more")) };
    let mut out = String::new();
    let mut step = |label: &str, changed: Result<bool, super::SettingsError>| {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "(no file)\n".to_string());
        out.push_str(&format!("# {label} -> changed: {}\n{text}", changed.unwrap()));
    };
    step("add_allow_rule", add_allow_rule(&project, rule));
    step("add_allow_rule again", add_allow_rule(&project, rule));
    step("add_directory", add_directory(&project, Path::new(dir)));
    step("save_always", save_always(&project, &always));
    step("remove_allow_rule", remove_allow_rule(&project, rule));
    step("remove_allow_rule absent", remove_allow_rule(&project, "Nope"));
    step("remove_allow_rule Bash(grep:*)", remove_allow_rule(&project, "Bash(grep:*)"));
    step("remove_allow_rule Edit", remove_allow_rule(&project, "Edit"));
    let _ = std::fs::remove_dir_all(&project);
    out
}

#[test]
fn golden_settings_local() {
    let typical = edit_steps("golden-a", None, "Bash(cargo test:*)", "/work/shared");
    let existing = "{\n    \"model\": \"opus\",\n    \"hooks\": {\"Stop\": [{\"command\": \"é\"}]},\n    \"permissions\": {\n        \"deny\": [\"Bash(rm:*)\"],\n        \"allow\": [\"Read\"],\n        \"defaultMode\": \"plan\"\n    }\n}";
    let odd = edit_steps("golden-b", Some(existing), "WebFetch(domain:docs.rs) \"é\"", "/work/with space/é");
    let actual = format!("## typical: no file yet\n{typical}## unusual: an existing file with other keys, deny list, four-space indent and no final newline\n{odd}");
    let golden = golden!("approval/settings_local.txt");
    assert_eq!(golden.split_once('\n').unwrap().1, actual);
}

#[test]
fn remembered_needs_every_rule_of_the_chain_saved() {
    let dir = tmp("remembered");
    let ps = Places { project: dir.clone(), home: None, temp: std::env::temp_dir(), disk: crate::domain::grantable::Disk(std::sync::Arc::new(crate::infrastructure::path_facts::SystemPathFacts)) };
    let input = bash("cargo build 2>&1 | tail -3");
    assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&input), None, &ps), None);
    add_allow_rule(&dir, "Bash(cargo build:*)").unwrap();
    assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&input), None, &ps), None, "tail is not saved yet");
    add_allow_rule(&dir, "Bash(tail:*)").unwrap();
    assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&input), None, &ps), Some(vec!["Bash(cargo build:*)".to_string(), "Bash(tail:*)".to_string()]));
    assert_eq!(remembered_with_local_trust("Bash", &tool_input_from_json(&input), None, &ps, false), None);
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(user_settings_path(&home), r#"{"permissions":{"allow":["Bash(cargo build:*)","Bash(tail:*)"]}}"#).unwrap();
    let user_rules = Places { home: Some(home), ..ps.clone() };
    assert_eq!(remembered_with_local_trust("Bash", &tool_input_from_json(&input), None, &user_rules, false), Some(vec!["Bash(cargo build:*)".to_string(), "Bash(tail:*)".to_string()]));
    assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&input), Some("/etc/x"), &ps), None, "a blocked path is never answered from rules");
    assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&bash("cargo build && rm x")), None, &ps), None);
    let plan = always_plan("Bash", &tool_input_from_json(&bash("cargo test | head")), None, &ps).unwrap();
    assert_eq!(plan.label(), "Bash(cargo test:*), Bash(head:*)");
    assert_eq!(save_always(&dir, &plan), Ok(true));
    assert!(load(&local_settings_path(&dir)).unwrap().list(List::Allow).contains(&"Bash(head:*)".to_string()));
}

#[test]
fn always_saves_the_rule_and_the_folder_in_one_write_or_neither() {
    let dir = tmp("save-always");
    let both = Always { rules: vec!["Bash(touch:*)".into()], folder: Some(PathBuf::from("/tmp/spike")) };
    assert_eq!(save_always(&dir, &both), Ok(true));
    let saved = load(&local_settings_path(&dir)).unwrap();
    assert_eq!((saved.list(List::Allow), saved.list(List::Directories)), (vec!["Bash(touch:*)".to_string()], vec!["/tmp/spike".to_string()]));
    assert_eq!(save_always(&dir, &both), Ok(false), "already there");
    // A settings file that cannot be written: neither part is saved, and it says so.
    let blocked = tmp("save-always-blocked");
    std::fs::write(blocked.join(".claude"), "x").unwrap();
    assert!(save_always(&blocked, &both).is_err());
    assert!(!local_settings_path(&blocked).exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&blocked);
}

#[test]
fn settings_round_trip_keeps_unknown_keys() {
    let dir = tmp("roundtrip");
    let path = local_settings_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"model":"x","hooks":{"a":[1,2]},"permissions":{"allow":["Bash(ls:*)"],"deny":["Read(.env)"],"defaultMode":"acceptEdits"}}"#,
    )
    .unwrap();
    assert!(add_allow_rule(&dir, "Bash(cargo test:*)").unwrap());
    assert!(add_directory(&dir, Path::new("/opt/data")).unwrap());
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["model"], "x");
    assert_eq!(v["hooks"], json!({"a": [1, 2]}));
    assert_eq!(v["permissions"]["deny"], json!(["Read(.env)"]));
    assert_eq!(v["permissions"]["defaultMode"], "acceptEdits");
    assert_eq!(v["permissions"]["allow"], json!(["Bash(ls:*)", "Bash(cargo test:*)"]));
    assert_eq!(v["permissions"]["additionalDirectories"], json!(["/opt/data"]));
    assert!(remove_allow_rule(&dir, "Bash(ls:*)").unwrap());
    assert_eq!(load(&path).unwrap().list(List::Allow), ["Bash(cargo test:*)"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rule_is_never_added_twice_or_removed_when_absent() {
    let dir = tmp("dupes");
    assert!(add_allow_rule(&dir, "Bash(ls:*)").unwrap(), "creates the file and the folder");
    assert!(!add_allow_rule(&dir, "Bash(ls:*)").unwrap());
    assert!(!remove_allow_rule(&dir, "Bash(nope:*)").unwrap());
    assert_eq!(load(&local_settings_path(&dir)).unwrap().list(List::Allow), ["Bash(ls:*)"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_cannot_be_read_back_is_refused_and_left_alone() {
    let dir = tmp("refuse");
    let path = local_settings_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for bad in [
        "{ not json",
        "[]",
        r#"{"permissions":[]}"#,
        r#"{"permissions":{"allow":"Bash"}}"#,
        r#"{"permissions":{"allow":[1]}}"#,
        r#"{"permissions":{"additionalDirectories":{}}}"#,
    ] {
        std::fs::write(&path, bad).unwrap();
        let e = add_allow_rule(&dir, "Bash(ls:*)").unwrap_err();
        assert!(e.0.contains("settings.local.json") && e.0.contains("left as it is"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad, "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_and_missing_files_are_empty_settings() {
    let dir = tmp("empty");
    let path = local_settings_path(&dir);
    assert_eq!(load(&path).unwrap(), Settings::empty());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "  \n").unwrap();
    assert_eq!(load(&path).unwrap(), Settings::empty());
    assert!(add_allow_rule(&dir, "WebSearch").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_file_is_private_even_when_it_was_not_and_no_temp_file_is_left() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp("mode");
    let path = local_settings_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    add_allow_rule(&dir, "WebSearch").unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dir_arguments_resolve_against_the_project_and_must_exist() {
    let dir = tmp("dirs");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("file"), "x").unwrap();
    assert_eq!(resolve_dir("sub", &dir, None).unwrap(), dir.join("sub").canonicalize().unwrap());
    assert_eq!(resolve_dir("~/sub", &dir, Some(&dir)).unwrap(), dir.join("sub").canonicalize().unwrap());
    assert_eq!(resolve_dir(dir.join("sub").to_str().unwrap(), Path::new("/nowhere"), None).unwrap(), dir.join("sub").canonicalize().unwrap());
    assert!(resolve_dir("missing", &dir, None).is_err());
    assert!(resolve_dir("file", &dir, None).unwrap_err().contains("not a folder"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_listing_reads_the_project_and_the_user_file_and_reports_a_broken_one() {
    let dir = tmp("listing");
    let home = tmp("listing-home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(user_settings_path(&home), r#"{"permissions":{"allow":["Bash(git status:*)"]}}"#).unwrap();
    add_allow_rule(&dir, "Bash(ls:*)").unwrap();
    add_directory(&dir, Path::new("/opt/x")).unwrap();
    let l = listing(&dir, Some(&home));
    assert_eq!((l.allow, l.directories, l.user_allow), (vec!["Bash(ls:*)".into()], vec!["/opt/x".into()], vec!["Bash(git status:*)".into()]));
    assert!(l.problems.is_empty());
    std::fs::write(user_settings_path(&home), "{").unwrap();
    let l = listing(&dir, Some(&home));
    assert!(l.user_allow.is_empty() && l.problems.len() == 1 && l.problems[0].contains("settings.json"), "{l:?}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn execution_flags_are_never_remembered() {
    for command in [
        "kubectl get pods --kubeconfig=./k.yaml",
        "kubectl get pods --kubeconfig ./k.yaml",
        "kubectl get pods --kubec=./k.yaml",
        "kubectl get pods '--kubeconfig=./k.yaml'",
        "kubectl get pods '--kube'config=./k.yaml",
        "helm list --kubeconfig=./k.yaml",
        "go test -toolexec=./evil ./...",
        "go test -vettool ./evil",
        "go test -exec=./evil",
        "go test '-tool'exec=./evil",
        "go test --toolexec=./evil",
        "echo ok | go test -exec=./evil",
    ] {
        assert!(rules_for("Bash", &tool_input_from_json(&bash(command)), None, &places()).is_err(), "{command}");
        assert_eq!(remembered(&SystemHost, "Bash", &tool_input_from_json(&bash(command)), None, &places()), None, "{command}");
        assert!(always_plan("Bash", &tool_input_from_json(&bash(command)), None, &places()).is_err(), "{command}");
    }
    assert!(rules_for("Bash", &tool_input_from_json(&bash("kubectl get pods --context=dev")), None, &places()).is_ok());
    assert!(rules_for("Bash", &tool_input_from_json(&bash("go test ./...")), None, &places()).is_ok());
}
