//! Scaffolding shared by the tests of this context: a scratch directory, a small repository,
//! and a fake turn.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::change_set::ChangeSummary;
use crate::domain::file_change::FileChange;
use crate::domain::turn_activity::TurnActivity;

/// A throwaway directory, removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("jev-changes-{}-{name}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(std::fs::canonicalize(p).unwrap())
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.0.join(rel)).unwrap()
    }
    pub fn exists(&self, rel: &str) -> bool {
        std::fs::symlink_metadata(self.0.join(rel)).is_ok()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A repo with a.txt (3 lines), b.txt, c.txt committed and `*.log` ignored.
pub fn repo() -> Scratch {
    let s = Scratch::new("repo");
    jev_testkit::git_repo::init_with_commit(
        s.path(),
        &[(".gitignore", "*.log\n"), ("a.txt", "one\ntwo\nthree\n"), ("b.txt", "bee\n"), ("src/c.txt", "see\n")],
    );
    s
}

/// What a turn did: modify a.txt, add new.txt, delete b.txt, and write an ignored file.
pub fn fake_turn(s: &Scratch) {
    s.write("a.txt", "one\nTWO\nthree\nfour\n");
    s.write("new.txt", "x\ny\nz\n");
    std::fs::remove_file(s.path().join("b.txt")).unwrap();
    s.write("debug.log", "noise\n");
}

pub fn files_by_path(sum: &ChangeSummary) -> HashMap<&str, &FileChange> {
    sum.files.iter().map(|f| (f.path.as_str(), f)).collect()
}

pub fn tools(lines: &[&str]) -> TurnActivity {
    TurnActivity::from_tools(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
}
