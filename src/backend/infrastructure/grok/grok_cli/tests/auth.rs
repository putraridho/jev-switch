use super::*;

#[test]
fn extra_env_reaches_the_child_so_a_key_kept_in_the_env_file_works() {
    let b = GrokBackend::new("grok").with_env(vec![("XAI_API_KEY".into(), "k".into())]);
    let cmd = b.command(&req(None));
    assert!(cmd.as_std().get_envs().any(|(k, v)| k == "XAI_API_KEY" && v == Some(std::ffi::OsStr::new("k"))));
}

#[test]
fn auth_env_picks_up_a_non_empty_key_only() {
    let lookup = |n: &str| (n == "XAI_API_KEY").then(|| "from-file".to_string());
    assert_eq!(auth_env(&lookup), vec![("XAI_API_KEY".to_string(), "from-file".to_string())]);
    assert!(auth_env(&|_| None).is_empty());
    assert!(auth_env(&|_| Some("  ".to_string())).is_empty());
}


#[test]
fn provider_is_grok() {
    assert_eq!(backend().provider(), ProviderId::Grok);
}

#[test]
fn session_dir_probe_checks_dot_grok_under_home() {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let home = std::env::temp_dir().join(format!("jev-grok-test-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    assert!(!session_dir_exists(&home));
    std::fs::create_dir(home.join(".grok")).unwrap();
    assert!(session_dir_exists(&home));
    std::fs::remove_dir_all(&home).unwrap();
}
