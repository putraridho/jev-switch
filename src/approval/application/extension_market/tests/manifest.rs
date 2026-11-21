use super::*;
use crate::approval::application::extension_market::installer::install;
use crate::approval::application::extension_market::manifest::{checked_path, pack};

#[cfg(unix)]
#[test]
fn pack_skips_links_to_folders() {
    let root = temp("loop");
    let src = root.join("src");
    package(&src, "1.0.0", "say hello");
    std::os::unix::fs::symlink("..", src.join("commands/up")).unwrap();
    let manifest = pack(&src, "hello-kit", "1.0.0").unwrap();
    assert_eq!(manifest.files.keys().collect::<Vec<_>>(), ["commands/hello.md"]);
    // Links out of the extension are not packed: a folder, a file, or a linked top folder.
    std::fs::create_dir_all(root.join("private")).unwrap();
    std::fs::write(root.join("private/key"), "secret").unwrap();
    std::os::unix::fs::symlink(root.join("private/key"), src.join("commands/key.md")).unwrap();
    std::os::unix::fs::symlink(root.join("private"), src.join("skills")).unwrap();
    std::os::unix::fs::symlink(root.join("private"), src.join("commands/private")).unwrap();
    let manifest = pack(&src, "hello-kit", "1.0.0").unwrap();
    assert_eq!(manifest.files.keys().collect::<Vec<_>>(), ["commands/hello.md"]);
}

#[test]
fn manifests_cannot_reach_outside_the_allowed_folders() {
    for bad in ["../evil.md", "/etc/passwd", "hooks/run.md", "commands", "commands/../../x"] {
        assert!(checked_path(bad).is_err(), "{bad}");
    }
    assert!(checked_path("skills/db/SKILL.md").is_ok());
    let root = temp("escape");
    std::fs::write(root.join(MANIFEST), r#"{"name":"x","version":"1","files":{"../escape.md":"00"}}"#).unwrap();
    assert!(install(&root.join("cfg"), &root).unwrap_err().0.contains("not allowed"));
    std::fs::write(root.join(MANIFEST), r#"{"name":"Bad Name","version":"1","files":{}}"#).unwrap();
    assert!(install(&root.join("cfg"), &root).unwrap_err().0.contains("not a usable extension name"));
}

#[test]
fn pack_needs_files_and_a_valid_name() {
    let root = temp("pack");
    assert!(pack(&root, "x", "1").unwrap_err().0.contains("nothing to pack"));
    std::fs::create_dir_all(root.join("recipes")).unwrap();
    std::fs::write(root.join("recipes/r.md"), "go").unwrap();
    assert!(pack(&root, "Bad", "1").is_err());
    assert_eq!(pack(&root, "ok", "2").unwrap().files.len(), 1);
}
