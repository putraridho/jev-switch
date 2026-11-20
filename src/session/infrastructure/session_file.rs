//! Where saved sessions live on disk, and reading, listing and pruning them. The files are
//! `<state dir>/sessions/<hash of the project folder>/<time>-<pid>.json`, one per session.

use std::path::{Path, PathBuf};

use super::session_meta::MetaStore;
use crate::session::domain::saved_session::{ResumeError, SavedSession, SessionEntry, SessionInfo};

/// The project folder as it is keyed on disk: canonical when it can be.
pub fn project_key(dir: &Path) -> String {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().into_owned()
}

/// FNV-1a, 64 bit: a hash that is the same on every machine and every Rust version,
/// which `DefaultHasher` does not promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// How many saved sessions are kept per project; the oldest go on save.
pub const SAVED_KEEP: usize = 10;

pub fn project_hash(project_dir: &Path) -> String {
    format!("{:016x}", fnv1a(project_key(project_dir).as_bytes()))
}

/// `<state_dir>/sessions/<hash of the canonical project folder>/`: one file per session.
pub fn project_sessions_dir(state_dir: &Path, project_dir: &Path) -> PathBuf {
    state_dir.join("sessions").join(project_hash(project_dir))
}

/// The single file older versions kept per project, `<state_dir>/sessions/<hash>.json`.
/// It is still found (and resumed) like any session of the project.
pub fn legacy_session_file(state_dir: &Path, project_dir: &Path) -> PathBuf {
    state_dir.join("sessions").join(format!("{}.json", project_hash(project_dir)))
}

/// A file for a session that starts now: `<project dir>/<now_ms>-<pid>.json`. A name that
/// is taken (two sessions in one millisecond of one process) moves on a millisecond, so a
/// new session never lands on, and overwrites, an older one.
pub fn new_session_file(state_dir: &Path, project_dir: &Path, now_ms: u64) -> PathBuf {
    let dir = project_sessions_dir(state_dir, project_dir);
    let pid = std::process::id();
    (now_ms..).map(|ms| dir.join(format!("{ms}-{pid}.json"))).find(|p| !p.exists()).expect("an unbounded range")
}

/// Read one session file. One that is not for `project_dir` (a hash collision) is
/// [`ResumeError::NotFound`].
fn read_session(path: &Path, project_dir: &Path) -> Result<SavedSession, ResumeError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ResumeError::NotFound),
        Err(e) => return Err(ResumeError::Io(e.to_string())),
    };
    let saved = SavedSession::parse(&text).map_err(ResumeError::Corrupt)?;
    if saved.project != project_key(project_dir) {
        return Err(ResumeError::NotFound);
    }
    Ok(saved)
}

/// Every session file of the project: those in its folder, and the old single file.
fn session_files(state_dir: &Path, project_dir: &Path) -> Vec<PathBuf> {
    let listed = std::fs::read_dir(project_sessions_dir(state_dir, project_dir))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"));
    let legacy = legacy_session_file(state_dir, project_dir);
    listed.chain(legacy.exists().then_some(legacy)).collect()
}

/// The most recently saved session of `project_dir` other than `current` (the file the
/// running session writes to). Sessions older than 14 days and files of other projects do
/// not count. `NotFound` when there is none; a damaged file is skipped unless it is all
/// there is, and then it is reported.
pub fn load_latest(
    state_dir: &Path,
    project_dir: &Path,
    now_ms: u64,
    current: Option<&Path>,
) -> Result<(PathBuf, SavedSession), ResumeError> {
    let mut best: Option<(PathBuf, SavedSession)> = None;
    let mut broken: Option<ResumeError> = None;
    for path in session_files(state_dir, project_dir).into_iter().filter(|p| Some(p.as_path()) != current) {
        match read_session(&path, project_dir) {
            Ok(s) if s.expired(now_ms) => {}
            // Equal times (a coarse clock): the later file name is the newer session.
            Ok(s) if best.as_ref().is_none_or(|(p, b)| (s.saved_ms, &path) > (b.saved_ms, p)) => best = Some((path, s)),
            Ok(_) | Err(ResumeError::NotFound) => {}
            Err(e) => broken = Some(e),
        }
    }
    best.ok_or(broken.unwrap_or(ResumeError::NotFound))
}

/// Every usable saved session of `project_dir` other than `current`, newest first: the
/// list `/resume` offers. Expired, empty, damaged and other projects' files are left out.
pub fn list_sessions(state_dir: &Path, project_dir: &Path, now_ms: u64, current: Option<&Path>) -> Vec<SessionEntry> {
    let marks = MetaStore::load(state_dir);
    let mut found: Vec<(u64, SessionEntry)> = session_files(state_dir, project_dir)
        .into_iter()
        .filter(|p| Some(p.as_path()) != current && !marks.is_archived(p))
        .filter_map(|path| {
            let saved = read_session(&path, project_dir).ok().filter(|s| !s.expired(now_ms) && s.sessions.turns() > 0)?;
            let title = saved.sessions.title().unwrap_or_else(|| "(no message)".to_string());
            Some((saved.saved_ms, SessionEntry { info: saved.info(), title, path }))
        })
        .collect();
    // Equal times (a coarse clock): the later file name is the newer session.
    found.sort_by(|(a, x), (b, y)| (b, &y.path).cmp(&(a, &x.path)));
    // Pinned sessions stay on top (the sort is stable, so each group keeps its order).
    found.sort_by_key(|(_, e)| !marks.is_pinned(&e.path));
    found.into_iter().map(|(_, e)| e).collect()
}

/// Load one session file of the project, as chosen from [`list_sessions`].
pub fn load_path(project_dir: &Path, path: &Path) -> Result<SavedSession, ResumeError> {
    read_session(path, project_dir)
}

/// Keep at most [`SAVED_KEEP`] sessions of the project: delete the oldest, never `current`.
/// A file that cannot be read counts as the oldest. Cheap while under the cap (a listing).
pub fn prune(state_dir: &Path, project_dir: &Path, current: &Path) {
    let marks = MetaStore::load(state_dir);
    // Pinned sessions are never deleted and do not count towards the cap.
    let files: Vec<PathBuf> = session_files(state_dir, project_dir).into_iter().filter(|p| !marks.is_pinned(p)).collect();
    if files.len() <= SAVED_KEEP {
        return;
    }
    let mut aged: Vec<(u64, PathBuf)> =
        files.into_iter().map(|p| (read_session(&p, project_dir).map_or(0, |s| s.saved_ms), p)).collect();
    aged.sort();
    let surplus = aged.len() - SAVED_KEEP;
    for (_, path) in aged.into_iter().filter(|(_, p)| p != current).take(surplus) {
        let _ = std::fs::remove_file(path);
    }
}

/// What there is to resume for `project_dir` in `state_dir` (its most recent session), if
/// anything usable.
pub fn saved_session_info_in(state_dir: &Path, project_dir: &Path, now_ms: u64) -> Option<SessionInfo> {
    load_latest(state_dir, project_dir, now_ms, None).ok().map(|(_, s)| s.info()).filter(|i| i.turns > 0)
}

/// [`saved_session_info_in`] for the default state directory and the real clock, for
/// the UI to offer resuming at startup.
pub fn saved_session_info(project_dir: &Path) -> Option<SessionInfo> {
    saved_session_info_in(&crate::shared::infrastructure::jsonl_log::state_dir()?, project_dir, crate::shared::infrastructure::jsonl_log::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::domain::session::Sessions;
    use crate::session::domain::turn_record::TurnRecord;
    use crate::session::domain::saved_session::SAVED_REPLY_CHARS;
    use crate::shared::infrastructure::private_file::write_atomic;
    use crate::shared::ProviderId;


    fn rec(provider: ProviderId, prompt: &str, reply: &str, tools: &[&str], ok: bool) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            provider,
            model: "m1".into(),
            reply: reply.into(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            ok,
        }
    }

    struct TempState(PathBuf);

    impl TempState {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("jev-sessions-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const NOW: u64 = 1_760_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn saved_for(dir: &Path, saved_ms: u64) -> SavedSession {
        let mut sessions = Sessions::new();
        sessions.set_session_id(ProviderId::Claude, "s1".into());
        sessions.record(rec(ProviderId::Claude, "hi", "hello", &[], true));
        SavedSession { project: project_key(dir), saved_ms, sessions, current: Some((ProviderId::Claude, "m1".into())) }
    }

    fn store(state: &Path, dir: &Path, saved: &SavedSession) -> PathBuf {
        let path = new_session_file(state, dir, saved.saved_ms);
        write_atomic(&path, &saved.to_json().to_string()).unwrap();
        path
    }

    #[test]
    fn a_saved_session_round_trips_through_its_file() {
        let (state, project) = (TempState::new("rt-state"), TempState::new("rt-proj"));
        let saved = saved_for(&project.0, NOW - 1000);
        store(&state.0, &project.0, &saved);
        let (_file, back) = load_latest(&state.0, &project.0, NOW, None).unwrap();
        assert_eq!((back.saved_ms, back.project.as_str()), (NOW - 1000, saved.project.as_str()));
        assert_eq!(back.current, Some((ProviderId::Claude, "m1".into())));
        assert_eq!(back.sessions.session_id(ProviderId::Claude), Some("s1"));
        let info = saved_session_info_in(&state.0, &project.0, NOW).unwrap();
        assert_eq!(info, SessionInfo { turns: 1, last_used_ms: NOW - 1000, providers: vec![ProviderId::Claude], current: back.current });
        // Written atomically and privately: no temp file left, owner-only on unix.
        let names: Vec<_> = std::fs::read_dir(project_sessions_dir(&state.0, &project.0)).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn a_garbage_or_foreign_file_is_an_error_not_a_session() {
        let (state, project) = (TempState::new("bad-state"), TempState::new("bad-proj"));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
        let file = new_session_file(&state.0, &project.0, NOW);
        for junk in ["not json at all", "[1,2,3]", "{}", r#"{"project":"x"}"#] {
            write_atomic(&file, junk).unwrap();
            assert!(matches!(load_latest(&state.0, &project.0, NOW, None), Err(ResumeError::Corrupt(_))), "{junk}");
            assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
        }
        // A file that says it is for another project is not ours.
        let mut other = saved_for(&project.0, NOW);
        other.project = "/somewhere/else".into();
        write_atomic(&file, &other.to_json().to_string()).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
    }

    #[test]
    fn sessions_older_than_fourteen_days_are_ignored() {
        let (state, project) = (TempState::new("age-state"), TempState::new("age-proj"));
        let file = store(&state.0, &project.0, &saved_for(&project.0, NOW - 13 * DAY));
        assert!(saved_session_info_in(&state.0, &project.0, NOW).is_some());
        write_atomic(&file, &saved_for(&project.0, NOW - 15 * DAY).to_json().to_string()).unwrap();
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
    }

    #[test]
    fn the_most_recent_session_other_than_the_current_one_is_found() {
        let (state, project) = (TempState::new("latest-state"), TempState::new("latest-proj"));
        let old = store(&state.0, &project.0, &saved_for(&project.0, NOW - 3 * DAY));
        let newer = store(&state.0, &project.0, &saved_for(&project.0, NOW - DAY));
        let expired = store(&state.0, &project.0, &saved_for(&project.0, NOW - 20 * DAY));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, newer);
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap().0, old, "the running session is skipped");
        std::fs::remove_file(&old).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap_err(), ResumeError::NotFound);
        assert!(expired.exists(), "an expired file is ignored, not deleted by a lookup");
    }

    #[test]
    fn the_old_single_file_is_still_found_and_ranks_by_its_time() {
        let (state, project) = (TempState::new("legacy-state"), TempState::new("legacy-proj"));
        let legacy = legacy_session_file(&state.0, &project.0);
        write_atomic(&legacy, &saved_for(&project.0, NOW - DAY).to_json().to_string()).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, legacy);
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW).unwrap().turns, 1);
        let newer = store(&state.0, &project.0, &saved_for(&project.0, NOW - 1000));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, newer);
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap().0, legacy);
    }

    #[test]
    fn a_damaged_file_is_skipped_when_a_good_one_exists() {
        let (state, project) = (TempState::new("skip-state"), TempState::new("skip-proj"));
        let good = store(&state.0, &project.0, &saved_for(&project.0, NOW - DAY));
        write_atomic(&project_sessions_dir(&state.0, &project.0).join("1-1.json"), "{ broken").unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, good);
    }

    #[test]
    fn new_session_files_never_collide() {
        let (state, project) = (TempState::new("collide-state"), TempState::new("collide-proj"));
        let first = store(&state.0, &project.0, &saved_for(&project.0, NOW));
        let second = new_session_file(&state.0, &project.0, NOW);
        assert_ne!(first, second, "a session started in the same millisecond must not take over the first one's file");
        assert_eq!(second.parent(), Some(project_sessions_dir(&state.0, &project.0).as_path()));
    }

    #[test]
    fn only_the_newest_ten_sessions_are_kept_and_the_current_one_never_goes() {
        let (state, project) = (TempState::new("prune-state"), TempState::new("prune-proj"));
        let files: Vec<PathBuf> = (0..13).map(|i| store(&state.0, &project.0, &saved_for(&project.0, NOW - (20 - i) * 1000))).collect();
        prune(&state.0, &project.0, &files[12]);
        let left = session_files(&state.0, &project.0);
        assert_eq!(left.len(), SAVED_KEEP);
        assert!(files[..3].iter().all(|f| !f.exists()), "the three oldest are gone");
        assert!(files[3..].iter().all(|f| f.exists()));
        // The current session is spared even when it is the oldest.
        let oldest = store(&state.0, &project.0, &saved_for(&project.0, NOW - 100_000));
        prune(&state.0, &project.0, &oldest);
        assert!(oldest.exists());
        assert_eq!(session_files(&state.0, &project.0).len(), SAVED_KEEP);
        assert!(!files[3].exists(), "the next oldest went instead");
    }

    #[test]
    fn each_project_has_its_own_file_and_the_name_is_stable() {
        let state = TempState::new("iso-state");
        let (a, b) = (TempState::new("iso-a"), TempState::new("iso-b"));
        assert_ne!(project_sessions_dir(&state.0, &a.0), project_sessions_dir(&state.0, &b.0));
        // A path spelled differently but the same folder keys the same file.
        let roundabout = a.0.join("..").join(a.0.file_name().unwrap());
        assert_eq!(project_sessions_dir(&state.0, &a.0), project_sessions_dir(&state.0, &roundabout));
        store(&state.0, &a.0, &saved_for(&a.0, NOW));
        assert!(saved_session_info_in(&state.0, &a.0, NOW).is_some());
        assert_eq!(saved_session_info_in(&state.0, &b.0, NOW), None);
        // FNV-1a test vector, so the file names never drift between releases.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_saved_session_without_turns_is_not_offered() {
        let (state, project) = (TempState::new("empty-state"), TempState::new("empty-proj"));
        let mut saved = saved_for(&project.0, NOW);
        saved.sessions = Sessions::new();
        store(&state.0, &project.0, &saved);
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
    }

    #[test]
    fn golden_saved_session() {
        let state = TempState::new("golden-state");
        let written = |name: &str, saved: &SavedSession| {
            let path = state.0.join(name);
            write_atomic(&path, &saved.render()).unwrap();
            std::fs::read_to_string(&path).unwrap()
        };
        let mut typical = Sessions::new();
        typical.set_session_id(ProviderId::Claude, "sess-1".into());
        typical.record(rec(ProviderId::Claude, "add a retry", "done", &["Edit src/a.rs", "Bash cargo test"], true));
        let typical = SavedSession { project: "/work/app".into(), saved_ms: NOW, sessions: typical, current: Some((ProviderId::Claude, "m1".into())) };

        let mut odd = Sessions::new();
        odd.set_name(Some("  the \"big\"\n refactor  "));
        odd.set_session_id(ProviderId::Codex, "sess-2".into());
        odd.record(rec(ProviderId::Codex, "ask é ✓\ttab", &"x".repeat(SAVED_REPLY_CHARS + 10), &[], true));
        odd.record_failed(rec(ProviderId::Gemini, "again", "(no answer: login)", &["Read b.rs"], false));
        odd.record_interrupted("stop it", ProviderId::Claude, "m2", vec![]);
        let odd = SavedSession { project: "/work/app \\ \"x\"".into(), saved_ms: NOW + 1, sessions: odd, current: None };

        let empty = SavedSession { project: "/work/empty".into(), saved_ms: 0, sessions: Sessions::new(), current: None };
        let actual = format!(
            "## typical: one answered turn\n{}\n## unusual: named, cut reply, failed and interrupted turns, escapes\n{}\n## unusual: nothing yet\n{}\n",
            written("a.json", &typical),
            written("b.json", &odd),
            written("c.json", &empty),
        );
        let golden = golden!("session/saved_session.txt");
        assert_eq!(golden.split_once('\n').unwrap().1, actual);
    }
}
