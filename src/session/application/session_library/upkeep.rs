//! Disk use, cleaning and migration of legacy session files.

use std::path::{Path, PathBuf};

use super::*;
use crate::session::application::ports::SessionStore;
use crate::session::domain::saved_session::SavedSession;
use crate::shared::application::host::Host;

// ------------------------------------------------------------ disk and clean

/// What the sessions take on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskUse {
    pub sessions: usize,
    pub bytes: u64,
    pub pinned: usize,
    pub archived: usize,
    /// `(project, sessions, bytes)`, biggest first.
    pub by_project: Vec<(String, usize, u64)>,
}

pub fn disk_use(entries: &[LibraryEntry]) -> DiskUse {
    let mut out = DiskUse::default();
    for e in entries {
        out.sessions += 1;
        out.bytes += e.bytes;
        out.pinned += usize::from(e.meta.pinned);
        out.archived += usize::from(e.meta.archived);
        match out.by_project.iter_mut().find(|(p, ..)| *p == e.saved.project) {
            Some((_, n, b)) => {
                *n += 1;
                *b += e.bytes;
            }
            None => out.by_project.push((e.saved.project.clone(), 1, e.bytes)),
        }
    }
    out.by_project.sort_by_key(|b| std::cmp::Reverse(b.2));
    out
}

/// `12.3 KB`.
pub fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / f64::from(1 << 10)),
        b => format!("{b} B"),
    }
}

/// Delete sessions last saved more than `older_than_days` ago, and every session file that
/// can no longer be read. Pinned sessions are never deleted. Returns how many files went
/// and how many bytes they were.
pub fn clean(store: &dyn SessionStore, state_dir: &Path, older_than_days: u64, now_ms: u64) -> (usize, u64) {
    let mut meta = store.load_marks(state_dir);
    let max_age_ms = older_than_days.saturating_mul(DAY_MS);
    let readable: std::collections::HashSet<PathBuf> = scan_all(store, state_dir).into_iter().map(|e| e.path).collect();
    let mut removed = (0, 0);
    let all_files = session_dirs(state_dir).into_iter().flat_map(|p| {
        if p.is_dir() {
            std::fs::read_dir(p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
        } else {
            vec![p]
        }
    });
    for path in all_files.filter(|p| p.extension().is_some_and(|x| x == "json")) {
        if meta.is_pinned(&path) {
            continue;
        }
        let old = SavedSession::parse(&std::fs::read_to_string(&path).unwrap_or_default())
            .map_or(true, |s| now_ms.saturating_sub(s.saved_ms) > max_age_ms || s.sessions.turns() == 0);
        if old || !readable.contains(&path) {
            let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
            if std::fs::remove_file(&path).is_ok() {
                meta.forget(&path);
                removed = (removed.0 + 1, removed.1 + bytes);
            }
        }
    }
    let _ = store.save_marks(state_dir, &meta);
    removed
}

// --------------------------------------------------------------- migration

/// Sessions saved by 0.2.0 and earlier sit as one file per project, `<hash>.json`, next to
/// the per-project folders. Move each into its project's folder (named by its save time)
/// so they list, search and prune like the rest. A file that cannot be read stays where it
/// is. Returns how many were moved; running it again moves nothing.
pub fn migrate_legacy(store: &dyn SessionStore, host: &dyn Host, state_dir: &Path) -> usize {
    let root = state_dir.join("sessions");
    let mut moved = 0;
    for path in std::fs::read_dir(&root).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
        if !path.is_file() || path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(saved) = std::fs::read_to_string(&path).ok().and_then(|t| SavedSession::parse(&t).ok()) else { continue };
        let dest_dir = store.project_sessions_dir(state_dir, Path::new(&saved.project));
        let dest = dest_dir.join(format!("{}-legacy.json", saved.saved_ms));
        let written = dest.exists() || host.write_private(&dest, &saved.render()).is_ok();
        if written && std::fs::remove_file(&path).is_ok() {
            moved += 1;
        }
    }
    moved
}
