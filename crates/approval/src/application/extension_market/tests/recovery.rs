use super::*;
use crate::application::extension_market::installer::install;

#[test]
fn a_lost_or_unreadable_registry_does_not_lock_out_jevs_own_copy() {
    for broken in [None, Some("{ not json")] {
        let root = temp(if broken.is_some() { "registry-corrupt" } else { "registry-lost" });
        let (src, user) = (root.join("src"), root.join("cfg"));
        package(&src, "1.0.0", "old");
        install(&SystemHost, &user, &src).unwrap();
        match broken {
            Some(text) => std::fs::write(user.join(REGISTRY), text).unwrap(),
            None => std::fs::remove_file(user.join(REGISTRY)).unwrap(),
        }
        package(&src, "2.0.0", "new");
        let installed = install(&SystemHost, &user, &src).unwrap();
        assert_eq!(installed.version, "2.0.0");
        assert_eq!(std::fs::read_to_string(user.join("extensions/hello-kit/commands/hello.md")).unwrap(), "new");
    }
}

#[test]
fn lost_registry_with_an_extra_file_preserves_the_draft() {
    let root = temp("lost-registry-draft");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::remove_file(user.join(REGISTRY)).unwrap();
    let draft = user.join("extensions/hello-kit/commands/draft.md");
    std::fs::write(&draft, "unfinished").unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert_eq!(std::fs::read_to_string(draft).unwrap(), "unfinished");
}

#[test]
fn lost_registry_with_only_system_junk_files_still_replaces_jevs_copy() {
    let root = temp("lost-registry-junk");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::remove_file(user.join(REGISTRY)).unwrap();
    let commands = user.join("extensions/hello-kit/commands");
    for junk in [".DS_Store", "._hello.md", "Thumbs.db", "desktop.ini"] {
        std::fs::write(commands.join(junk), "junk").unwrap();
    }
    package(&src, "2.0.0", "new");

    let installed = install(&SystemHost, &user, &src).unwrap();
    assert_eq!(installed.version, "2.0.0");
    assert_eq!(std::fs::read_to_string(commands.join("hello.md")).unwrap(), "new");
}

#[test]
fn lost_registry_with_a_modified_file_preserves_the_edit() {
    let root = temp("lost-registry-edit");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::remove_file(user.join(REGISTRY)).unwrap();
    let edited = user.join("extensions/hello-kit/commands/hello.md");
    std::fs::write(&edited, "my changes").unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert_eq!(std::fs::read_to_string(edited).unwrap(), "my changes");
}

#[test]
fn lost_registry_with_a_missing_file_refuses_replacement() {
    let root = temp("lost-registry-missing");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::remove_file(user.join(REGISTRY)).unwrap();
    let missing = user.join("extensions/hello-kit/commands/hello.md");
    std::fs::remove_file(&missing).unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert!(!missing.exists());
}

#[cfg(unix)]
#[test]
fn lost_registry_with_a_link_refuses_replacement() {
    let root = temp("lost-registry-link");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::remove_file(user.join(REGISTRY)).unwrap();
    let link = user.join("extensions/hello-kit/commands/link.md");
    std::os::unix::fs::symlink("hello.md", &link).unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert!(std::fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

#[test]
fn copied_manifest_does_not_make_a_handmade_folder_managed() {
    let root = temp("copied-manifest");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "original");
    let target = user.join("extensions/hello-kit");
    std::fs::create_dir_all(target.join("commands")).unwrap();
    std::fs::copy(src.join(MANIFEST), target.join(MANIFEST)).unwrap();
    std::fs::copy(src.join("commands/hello.md"), target.join("commands/hello.md")).unwrap();
    std::fs::write(target.join("notes.txt"), "my notes").unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert_eq!(std::fs::read_to_string(target.join("notes.txt")).unwrap(), "my notes");
}
