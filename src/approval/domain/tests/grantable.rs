use super::*;

#[test]
fn unsafe_home_or_temp_folders_make_nothing_grantable_through_them() {
    let with = |home: Option<&str>, temp: &str| Places { project: project().to_path_buf(), home: home.map(PathBuf::from), temp: PathBuf::from(temp) };
    for (home, dir) in [
        ("/", "/etc/ssh"), ("/", "/usr/bin"), ("/home", "/home/other/x"), ("/root", "/root/x"), ("home/u", "home/u/x"),
        ("/etc/x", "/etc/x/y"), ("/usr/local", "/usr/local/bin"), ("/home/u/..", "/home/x"),
    ] {
        assert!(!grantable_in(Path::new(dir), Some(Path::new(home)), Path::new("/tmp")), "HOME={home} must not grant {dir}");
        assert_eq!(grantable_folder(Path::new(dir), &with(Some(home), "/tmp")), Err(Refusal::FolderTooBroad), "HOME={home} {dir}");
    }
    for (temp, dir) in [
        ("/", "/etc/ssh"), ("/home", "/home/other/x"), ("/var", "/var/lib/x"), ("/usr/bin", "/usr/bin/x"), ("/etc", "/etc/ssh"),
        ("tmp", "tmp/x"), ("/home/u", "/home/u/.ssh"), ("/home", "/home/u/.ssh"), ("/Users", "/Users/u/.ssh"),
    ] {
        assert!(!grantable_in(Path::new(dir), Some(Path::new("/home/u")), Path::new(temp)), "TMPDIR={temp} must not grant {dir}");
    }
    // The usual places still work.
    assert!(grantable_in(Path::new("/tmp/x"), Some(Path::new("/home/u")), Path::new("/tmp")));
    assert!(grantable_in(Path::new("/var/folders/ab/T/x"), None, Path::new("/var/folders/ab/T")));
    assert!(grantable_in(Path::new("/home/u/tmp/x"), Some(Path::new("/home/u")), Path::new("/home/u/tmp")));
    assert!(grantable_in(Path::new("/Users/u/code"), Some(Path::new("/Users/u")), Path::new("/tmp")));
}

#[test]
fn a_read_family_rule_outside_the_project_never_covers_a_broad_or_secret_folder() {
    let rule = |tool: &str, input: Value| rule_for(tool, &input, None, &places());
    for (tool, input) in [
        // `~/.bashrc` would have saved `Read(//home/u/**)`: ~/.ssh and ~/.aws with it.
        ("Read", json!({"file_path": "/home/u/.bashrc"})),
        ("Read", json!({"file_path": "/home/u/.ssh/config"})),
        ("Read", json!({"file_path": "/home/u/.aws/credentials"})),
        ("Read", json!({"file_path": "/home/u/.config/gh/hosts.yml"})),
        ("Read", json!({"file_path": "/home/u/.local/share/keyrings/login.keyring"})),
        ("Read", json!({"file_path": "/home/u/work/.git/config"})),
        // The root, system folders, another user's home, the temp folder itself.
        ("Grep", json!({"pattern": "x", "path": "/"})),
        ("LS", json!({"path": "/"})),
        ("Glob", json!({"pattern": "*", "path": "/etc"})),
        ("Read", json!({"file_path": "/etc/ssh/sshd_config"})),
        ("Read", json!({"file_path": "/usr/bin/ls"})),
        ("Read", json!({"file_path": "/home/other/notes.txt"})),
        ("Read", json!({"file_path": "/tmp/x.txt"})),
        ("LS", json!({"path": "/home/u"})),
        ("LS", json!({"path": "/home"})),
    ] {
        assert_eq!(rule(tool, input.clone()), Err(Refusal::FolderTooBroad), "{tool} {input}");
    }
    // With no home folder known, nothing outside the project but the temp folder qualifies.
    assert_eq!(rule_for("Read", &json!({"file_path": "/home/u/notes/a"}), None, &places_with(None)), Err(Refusal::FolderTooBroad));
}
