//! Temporary git repositories for tests: run `git` in a directory with a fixed identity and
//! signing off (panicking on failure), and make a repository with a first commit.

use std::path::Path;
use std::process::Command;

/// `git -C dir args...` with the identity `t <t@t>` and `commit.gpgsign=false`; returns stdout.
/// Panics, with git's stderr, when git fails.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `git init -q` in `dir`, write each `(relative path, text)` file, and commit them all as "init".
pub fn init_with_commit(dir: &Path, files: &[(&str, &str)]) {
    git(dir, &["init", "-q"]);
    for (rel, text) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}
