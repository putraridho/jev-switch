//! The files and index of a tree, taken before a risky apply so they can be put back.

use std::path::{ Component, Path, PathBuf };

/// Files and the index as they were, so a failed merge can put the tree back exactly.
/// `git apply --3way` writes conflict markers into the working tree and stages partial
/// results when it fails, which is not "leave the tree untouched".
pub(super) struct Snapshot {
    index: Option<(PathBuf, Vec<u8>)>,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Snapshot {
    pub(super) fn take(repo: &Path, index: Option<PathBuf>, paths: &[String]) -> Self {
        let safe = |p: &&String|
            Path::new(p)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        let files = paths
            .iter()
            .filter(safe)
            .map(|p| repo.join(p))
            .map(|p| {
                let bytes = std::fs::read(&p).ok();
                (p, bytes)
            });
        Self {
            index: index.and_then(|p|
                std::fs
                    ::read(&p)
                    .ok()
                    .map(|b| (p, b))
            ),
            files: files.collect(),
        }
    }

    pub(super) fn restore(&self) {
        for (path, bytes) in &self.files {
            match bytes {
                Some(b) => {
                    let _ = std::fs::write(path, b);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        if let Some((path, bytes)) = &self.index {
            let _ = std::fs::write(path, bytes);
        }
    }
}
