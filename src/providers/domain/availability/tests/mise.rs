//! Never running a lazy launcher, and finding a real install from the filesystem alone.

use std::path::{Path, PathBuf};

use crate::providers::domain::availability::host::{find_on_path, probe_command};
use crate::providers::domain::availability::mise::{is_mise_wrapper, mise_data_dir, mise_installs_binary, really_installed};

#[test]
fn probes_run_in_mise_offline_mode_so_they_never_update_or_install() {
    let cmd = probe_command("codex", &["login", "status"]);
    let env: Vec<_> = cmd.as_std().get_envs().collect();
    assert!(env.iter().any(|(k, v)| *k == "MISE_OFFLINE" && *v == Some(std::ffi::OsStr::new("1"))), "{env:?}");
}

#[test]
fn lazy_launchers_are_never_counted_as_installed() {
    let wrapper = PathBuf::from("/h/.local/share/mise/command-wrappers/bin/codex");
    let shim = PathBuf::from("/h/.local/share/mise/shims/agy");
    let real = PathBuf::from("/usr/bin/codex");
    assert!(is_mise_wrapper(&shim), "mise shims install on first run too (the agy accident)");
    assert!(!is_mise_wrapper(Path::new("/opt/shims/tool")), "only mise's shims directory");
    // Only a lazy launcher and mise has nothing installed: never run it.
    assert!(!really_installed(std::slice::from_ref(&wrapper), false));
    assert!(!really_installed(std::slice::from_ref(&shim), false));
    // A lazy launcher for a tool mise already installed is safe to run.
    assert!(really_installed(std::slice::from_ref(&wrapper), true));
    // A real binary anywhere on PATH is installed.
    assert!(really_installed(&[shim, real], false));
    // Nothing on PATH is never installed, whatever mise says.
    assert!(!really_installed(&[], true));
}

/// A mise data directory in a fresh temp folder.
fn mise_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jev-mise-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("installs")).unwrap();
    dir
}

#[cfg(unix)]
fn write_file(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn write_file(path: &Path, _mode: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\n").unwrap();
}

#[test]
fn an_install_is_found_by_its_binary_whatever_the_tool_directory_is_called() {
    let data = mise_dir("layouts");
    // The directory is named like the binary, the binary in the version directory.
    write_file(&data.join("installs/claude/2.1.3/claude"), 0o755);
    assert!(mise_installs_binary(&data, "claude"));
    // Named like the binary, in `bin/`.
    write_file(&data.join("installs/codex/0.1.0/bin/codex"), 0o755);
    assert!(mise_installs_binary(&data, "codex"));
    // A backend-prefixed tool name: `npm:@google/gemini-cli` is `npm-google-gemini-cli`.
    write_file(&data.join("installs/npm-google-gemini-cli/0.9.0/bin/gemini"), 0o755);
    assert!(mise_installs_binary(&data, "gemini"));
    // A binary that differs from the tool (`agy` from `antigravity-cli`).
    write_file(&data.join("installs/antigravity-cli/1.0.0/bin/agy"), 0o755);
    assert!(mise_installs_binary(&data, "agy"));
    // Other binaries are not found just because something is installed.
    assert!(!mise_installs_binary(&data, "grok"));
    assert!(!mise_installs_binary(&data.join("missing"), "claude"));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn an_empty_version_directory_or_a_non_executable_file_is_not_an_install() {
    let data = mise_dir("hollow");
    // `mise install` started and did not finish (it happened with grok): a version
    // directory exists, with nothing in it.
    std::fs::create_dir_all(data.join("installs/grok/1.0.77")).unwrap();
    assert!(!mise_installs_binary(&data, "grok"));
    // Files that are not the binary.
    write_file(&data.join("installs/grok/1.0.77/README"), 0o644);
    assert!(!mise_installs_binary(&data, "grok"));
    // The right name but not executable.
    write_file(&data.join("installs/gemini/0.1.0/bin/gemini"), 0o644);
    assert!(!mise_installs_binary(&data, "gemini"));
    // A directory with the binary's name is not a binary either.
    std::fs::create_dir_all(data.join("installs/agy/1.0.0/bin/agy")).unwrap();
    assert!(!mise_installs_binary(&data, "agy"));
    // And once it really is installed, it counts.
    write_file(&data.join("installs/grok/1.0.77/bin/grok"), 0o755);
    assert!(mise_installs_binary(&data, "grok"));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn mise_wrapper_paths_are_recognised() {
    assert!(is_mise_wrapper(Path::new("/h/.local/share/mise/command-wrappers/bin/grok")));
    assert!(!is_mise_wrapper(Path::new("/h/.local/share/mise/installs/grok/1.0/bin/grok")));
    assert!(!is_mise_wrapper(Path::new("/usr/bin/grok")));
}

#[test]
fn mise_data_dir_precedence() {
    let h = Some(PathBuf::from("/h"));
    assert_eq!(mise_data_dir(Some("/m".into()), Some("/x".into()), h.clone()), Some(PathBuf::from("/m")));
    assert_eq!(mise_data_dir(None, Some("/x".into()), h.clone()), Some(PathBuf::from("/x/mise")));
    assert_eq!(mise_data_dir(None, None, h), Some(PathBuf::from("/h/.local/share/mise")));
    assert_eq!(mise_data_dir(None, None, None), None);
}

// ---- PATH search

#[test]
fn find_on_path_returns_matches_in_order() {
    let dirs = [PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/c")];
    let exists = |p: &Path| p == Path::new("/b/tool") || p == Path::new("/c/tool");
    assert_eq!(find_on_path("tool", &dirs, &exists), vec![PathBuf::from("/b/tool"), PathBuf::from("/c/tool")]);
    assert!(find_on_path("other", &dirs, &exists).is_empty());
}
