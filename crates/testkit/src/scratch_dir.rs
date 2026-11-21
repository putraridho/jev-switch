//! A scratch directory under the system temp dir for a test: named by a crate-wide prefix
//! and a per-test name, emptied and created fresh, so a test starts from nothing.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// `<tmp>/jev-<prefix>-<name>-<pid>`, removed if it exists and created empty.
pub fn scratch_dir(prefix: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jev-{prefix}-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Like [`scratch_dir`], with a counter in the name so a test can ask for many.
pub fn unique_scratch_dir(prefix: &str, name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    scratch_dir(prefix, &format!("{name}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}
