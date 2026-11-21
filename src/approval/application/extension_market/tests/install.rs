use super::*;
use crate::approval::application::extension_market::installer::{install, remove, Installed};
use crate::approval::application::extension_market::listing::{list, State};
use crate::approval::application::extension_market::manifest::read_manifest;
use crate::shared::domain::sha256;

#[cfg(unix)]
#[test]
fn install_refuses_files_reached_through_links() {
    let root = temp("links");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "say hello");
    // The manifest is genuine, then the file is swapped for a link to a file with the same text.
    std::fs::write(root.join("outside.md"), "say hello").unwrap();
    std::fs::remove_file(src.join("commands/hello.md")).unwrap();
    std::os::unix::fs::symlink(root.join("outside.md"), src.join("commands/hello.md")).unwrap();
    let done = install(&SystemHost, &user, &src);
    assert!(done.as_ref().is_err_and(|e| e.0.contains("symbolic link")), "{done:?}");
    assert!(installed_dirs(&user).is_empty());
    // A linked folder in front of the file is refused too.
    std::fs::remove_file(src.join("commands/hello.md")).unwrap();
    std::fs::remove_dir(src.join("commands")).unwrap();
    std::fs::create_dir_all(root.join("elsewhere")).unwrap();
    std::fs::write(root.join("elsewhere/hello.md"), "say hello").unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), src.join("commands")).unwrap();
    let done = install(&SystemHost, &user, &src);
    assert!(done.as_ref().is_err_and(|e| e.0.contains("symbolic link")), "{done:?}");
}

#[cfg(unix)]
#[test]
fn install_refuses_a_manifest_that_is_a_link() {
    let root = temp("manifest-link");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "say hello");
    std::fs::rename(src.join(MANIFEST), root.join("elsewhere.json")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere.json"), src.join(MANIFEST)).unwrap();
    let done = install(&SystemHost, &user, &src);
    assert!(done.as_ref().is_err_and(|e| e.0.contains("symbolic link")), "{done:?}");
    assert!(installed_dirs(&user).is_empty());
}

#[test]
fn a_failed_update_keeps_the_installed_copy() {
    let root = temp("keep");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "say hello");
    install(&SystemHost, &user, &src).unwrap();
    // A file is in the way of the staged folder's name, so staging cannot be written.
    std::fs::write(user.join("extensions/.hello-kit.new"), "x").unwrap();
    package(&src, "2.0.0", "say hi");
    let done = install(&SystemHost, &user, &src);
    assert!(done.is_err(), "{done:?}");
    assert_eq!(std::fs::read_to_string(user.join("extensions/hello-kit/commands/hello.md")).unwrap(), "say hello");
    assert_eq!(installed_dirs(&user), vec![user.join("extensions/hello-kit")]);
}

#[test]
fn install_refuses_to_replace_an_unmanaged_extension() {
    let root = temp("unmanaged");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "packaged");
    let target = user.join("extensions/hello-kit");
    std::fs::create_dir_all(target.join("commands")).unwrap();
    std::fs::write(target.join("commands/hello.md"), "hand-written").unwrap();
    std::fs::write(target.join("notes.txt"), "keep me").unwrap();

    let error = install(&SystemHost, &user, &src).unwrap_err();
    assert!(error.0.contains("was not installed from a package"), "{error}");
    assert!(error.0.contains("remove or rename"), "{error}");
    assert_eq!(std::fs::read_to_string(target.join("commands/hello.md")).unwrap(), "hand-written");
    assert_eq!(std::fs::read_to_string(target.join("notes.txt")).unwrap(), "keep me");
    assert!(list(&user).is_empty());
}

#[test]
fn install_replaces_a_registry_managed_extension() {
    let root = temp("managed-replace");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "old");
    install(&SystemHost, &user, &src).unwrap();
    std::fs::write(user.join("extensions/hello-kit/commands/hello.md"), "edited").unwrap();
    std::fs::write(user.join("extensions/hello-kit/commands/draft.md"), "draft").unwrap();
    package(&src, "2.0.0", "new");

    let installed = install(&SystemHost, &user, &src).unwrap();
    assert_eq!(installed.replaced.as_deref(), Some("1.0.0"));
    assert_eq!(installed.version, "2.0.0");
    assert_eq!(std::fs::read_to_string(user.join("extensions/hello-kit/commands/hello.md")).unwrap(), "new");
    assert_eq!(list(&user)[0].state, State::Verified);
}

#[test]
fn pack_install_list_and_remove() {
    let root = temp("flow");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "say hello");
    let manifest = read_manifest(&src).unwrap();
    assert_eq!(manifest.files["commands/hello.md"], sha256::hex(b"say hello"));

    let done = install(&SystemHost, &user, &src).unwrap();
    assert_eq!(done, Installed { name: "hello-kit".into(), version: "1.0.0".into(), files: 1, replaced: None });
    assert!(user.join("extensions/hello-kit/commands/hello.md").exists());
    assert_eq!(installed_dirs(&user), vec![user.join("extensions/hello-kit")]);
    let listed = list(&user);
    assert_eq!((listed.len(), &listed[0].state), (1, &State::Verified));

    remove(&SystemHost, &user, "hello-kit").unwrap();
    assert!(list(&user).is_empty() && installed_dirs(&user).is_empty());
    assert!(remove(&SystemHost, &user, "hello-kit").unwrap_err().0.contains("not installed"));
}

#[test]
fn a_file_that_does_not_match_its_checksum_installs_nothing() {
    let root = temp("tamper");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "say hello");
    std::fs::write(src.join("commands/hello.md"), "say hello; and exfiltrate").unwrap();
    let err = install(&SystemHost, &user, &src).unwrap_err().0;
    assert!(err.contains("checksum mismatch for commands/hello.md"), "{err}");
    assert!(installed_dirs(&user).is_empty() && list(&user).is_empty());
}
