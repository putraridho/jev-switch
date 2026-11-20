//! Saving a session after every turn, and resuming one.

use super::*;
use crate::session::infrastructure::session_file;

#[tokio::test]
async fn new_session_forgets_the_conversation_but_not_the_pin_or_cooldowns() {
    let mut r = rig(&[Claude]);
    r.turn("one", want(TaskType::Feature)).await;
    r.engine.set_pin(Pin::Provider(Claude));
    r.engine.runtime.cool_down(Codex, None, NOW);
    r.engine.new_session();
    assert!(r.engine.sessions.recent_context(3, 100).is_empty());
    assert!(r.engine.routing.current.is_none());
    assert_eq!(r.engine.pin(), &Pin::Provider(Claude));
    assert!(r.engine.runtime.cooldowns.active(Codex, NOW).is_some());
    r.turn("two", want(TaskType::Feature)).await;
    assert_eq!(r.started().last().unwrap().session, None, "a fresh session resumes nothing");
}

// ------------------------------------------------------------------ saved sessions

struct StateDir(PathBuf);

impl StateDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("jev-turn-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn persisting(repo: &Repo, state: &StateDir, ready: &[ProviderId]) -> Rig {
    let mut r = rig_in(repo.0.clone(), &states_for(ready), OK);
    r.engine = r.engine.with_persistence(Some(state.0.clone()));
    r
}

#[tokio::test]
async fn a_session_is_saved_after_every_turn_and_resumed_by_a_new_engine() {
    let (repo, state) = (Repo::new("persist", false), StateDir::new("persist"));
    let mut r = persisting(&repo, &state, &[Claude]);
    assert!(r.engine.save_now().is_ok(), "nothing to save yet is not an error");
    assert_eq!(r.engine.session_path(), None, "nothing saved yet, so no file yet");
    r.turn("first request", want(TaskType::Feature)).await;
    r.turn("second request", want(TaskType::Feature)).await;
    let file = r.engine.session_path().unwrap().to_path_buf();
    assert!(file.starts_with(session_file::project_sessions_dir(&state.0, &repo.0)) && file.exists(), "{file:?}");

    // A fresh process: nothing in memory, same folder.
    let mut again = persisting(&repo, &state, &[Claude]);
    assert_eq!(again.engine.sessions.turns(), 0);
    let info = again.engine.resume().unwrap();
    assert_eq!((info.turns, info.providers.clone(), info.last_used_ms), (2, vec![Claude], NOW));
    assert_eq!(info.current.as_ref().map(|c| c.0), Some(Claude));
    assert_eq!(again.engine.routing.current, r.engine.routing.current);
    assert_eq!(again.engine.sessions.session_id(Claude), Some("sess-claude"));
    assert_eq!(again.engine.sessions.recent_context(2, 50), r.engine.sessions.recent_context(2, 50));
    assert_eq!(session_file::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 2);

    // The CLI session is continued, not restarted.
    again.turn("third request", want(TaskType::Feature)).await;
    assert_eq!(again.started()[0].session.as_deref(), Some("sess-claude"));
    assert_eq!(again.engine.sessions.turns(), 3);
}

#[tokio::test]
async fn cancelled_turns_are_saved_too() {
    let (repo, state) = (Repo::new("persist-cancel", false), StateDir::new("persist-cancel"));
    let mut r = persisting(&repo, &state, &[Claude]);
    r.script(Claude, HANGS);
    let cancel = Cancel::new();
    r.ui.cancel_on_tool = Some(cancel.clone());
    let judged = Judged::Providers(Some(want(TaskType::Feature)));
    r.engine.run_judged_cancellable("start it", judged, &mut r.ui, &cancel).await;
    let mut again = persisting(&repo, &state, &[Claude]);
    assert_eq!(again.engine.resume().unwrap().turns, 1);
    assert!(again.engine.sessions.catch_up(Codex, "", 1000).unwrap().contains("interrupted by the user"));
}

#[tokio::test]
async fn persistence_is_off_unless_asked_for() {
    let repo = Repo::new("persist-off", false);
    let mut r = rig_in(repo.0.clone(), &states_for(&[Claude]), OK);
    r.turn("hello", want(TaskType::Feature)).await;
    assert_eq!(r.engine.session_path(), None);
    assert!(r.engine.save_now().is_ok());
    assert_eq!(r.engine.resume().unwrap_err(), ResumeError::Disabled);
}

#[tokio::test]
async fn new_session_starts_another_file_and_leaves_the_old_one_resumable() {
    let (repo, state) = (Repo::new("persist-new", false), StateDir::new("persist-new"));
    let mut r = persisting(&repo, &state, &[Claude]);
    r.turn("hello", want(TaskType::Feature)).await;
    let old = r.engine.session_path().unwrap().to_path_buf();
    r.engine.new_session();
    assert!(old.exists(), "/new no longer deletes the saved session");
    assert_eq!(r.engine.session_path(), None);
    r.turn("something else", want(TaskType::Feature)).await;
    let new = r.engine.session_path().unwrap().to_path_buf();
    assert_ne!(old, new);
    assert_eq!(r.engine.sessions.turns(), 1, "the new session starts empty");
    // The old one is what /resume brings back.
    assert_eq!(r.engine.resume().unwrap().turns, 1);
    assert_eq!(r.engine.session_path(), Some(old.as_path()));
    assert!(r.engine.sessions.recent_context(1, 50).contains("hello"));
}

/// A session of one prompt, saved by an engine of its own (a process of its own).
async fn session_of(repo: &Repo, state: &StateDir, prompts: &[&str], at: u64) -> Rig {
    let mut r = persisting(repo, state, &[Claude]);
    r.clock.store(at, Ordering::SeqCst);
    for p in prompts {
        r.turn(p, want(TaskType::Feature)).await;
    }
    r
}

#[tokio::test]
async fn a_new_sessions_first_turn_does_not_overwrite_the_previous_one() {
    let (repo, state) = (Repo::new("persist-keep", false), StateDir::new("persist-keep"));
    let first = session_of(&repo, &state, &["one", "two", "three"], NOW).await;
    assert_eq!(session_file::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 3);

    // The startup offer was `previous session: 3 turns`; the user ignores it and types.
    let second = session_of(&repo, &state, &["a new topic"], NOW + 1000).await;
    assert_ne!(first.engine.session_path(), second.engine.session_path());

    // The 3-turn conversation is intact: it is what is left after the newer one.
    let mut third = persisting(&repo, &state, &[Claude]);
    third.clock.store(NOW + 2000, Ordering::SeqCst);
    assert_eq!(third.engine.resume().unwrap().turns, 1, "the most recent session comes first");
    assert_eq!(third.engine.resume().unwrap().turns, 3, "and the one before it was not destroyed");
    assert!(third.engine.sessions.recent_context(3, 50).contains("three"));
}

#[tokio::test]
async fn resuming_continues_the_resumed_file_and_a_further_resume_steps_back() {
    let (repo, state) = (Repo::new("persist-step", false), StateDir::new("persist-step"));
    session_of(&repo, &state, &["old one", "old two"], NOW).await;
    session_of(&repo, &state, &["recent"], NOW + 1000).await;
    let mut r = persisting(&repo, &state, &[Claude]);
    r.clock.store(NOW + 2000, Ordering::SeqCst);
    r.engine.resume().unwrap();
    let recent = r.engine.session_path().unwrap().to_path_buf();
    r.turn("and more", want(TaskType::Feature)).await;
    assert_eq!(r.engine.session_path(), Some(recent.as_path()), "a resumed session is continued in place, not copied");
    assert_eq!(std::fs::read_dir(recent.parent().unwrap()).unwrap().count(), 2, "no third file");
    assert_eq!(r.engine.resume().unwrap().turns, 2, "the older session is next; the running one is skipped");
    assert_ne!(r.engine.session_path(), Some(recent.as_path()));
    // With only two sessions, the running one is always skipped, so it alternates.
    r.engine.resume().unwrap();
    assert_eq!(r.engine.session_path(), Some(recent.as_path()));
}

#[tokio::test]
async fn the_resume_list_is_newest_first_named_by_the_first_message_and_any_can_be_picked() {
    let (repo, state) = (Repo::new("persist-list", false), StateDir::new("persist-list"));
    session_of(&repo, &state, &["fix the parser", "and its tests"], NOW).await;
    session_of(&repo, &state, &["rename the crate"], NOW + 1000).await;
    let mut r = persisting(&repo, &state, &[Claude]);
    r.clock.store(NOW + 2000, Ordering::SeqCst);
    let list = r.engine.resumable().unwrap();
    let shown: Vec<_> = list.iter().map(|e| (e.title.as_str(), e.info.turns)).collect();
    assert_eq!(shown, vec![("rename the crate", 1), ("fix the parser", 2)]);
    // The older one, not the newest, is taken; and it is continued in place.
    let info = r.engine.resume_from(&list[1].path).unwrap();
    assert_eq!(info.turns, 2);
    assert_eq!(r.engine.session_path(), Some(list[1].path.as_path()));
    // The running session is not offered again.
    let after: Vec<_> = r.engine.resumable().unwrap().iter().map(|e| e.title.clone()).collect();
    assert_eq!(after, vec!["rename the crate".to_string()]);
}

#[tokio::test]
async fn the_resume_list_leaves_out_other_folders_and_missing_files_fail_cleanly() {
    let (repo, other, state) = (Repo::new("persist-list-a", false), Repo::new("persist-list-b", false), StateDir::new("persist-list-ab"));
    session_of(&other, &state, &["someone else's work"], NOW).await;
    let mut r = persisting(&repo, &state, &[Claude]);
    assert!(r.engine.resumable().unwrap().is_empty());
    let gone = state.0.join("sessions").join("nothing.json");
    assert_eq!(r.engine.resume_from(&gone).unwrap_err(), ResumeError::NotFound);
}

#[tokio::test]
async fn only_the_ten_newest_sessions_of_a_project_are_kept() {
    let (repo, state) = (Repo::new("persist-cap", false), StateDir::new("persist-cap"));
    let mut files = Vec::new();
    for i in 0..12u64 {
        let r = session_of(&repo, &state, &["hello"], NOW + i * 1000).await;
        files.push(r.engine.session_path().unwrap().to_path_buf());
    }
    let dir = session_file::project_sessions_dir(&state.0, &repo.0);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), session_file::SAVED_KEEP);
    assert!(!files[0].exists() && !files[1].exists(), "the oldest went");
    assert!(files[2..].iter().all(|f| f.exists()));
}

#[tokio::test]
async fn a_session_saved_by_an_older_version_in_one_file_is_still_resumed() {
    let (repo, state) = (Repo::new("persist-legacy", false), StateDir::new("persist-legacy"));
    let donor = session_of(&repo, &state, &["from before"], NOW).await;
    // Move that session to where the old version kept it.
    let legacy = session_file::legacy_session_file(&state.0, &repo.0);
    std::fs::rename(donor.engine.session_path().unwrap(), &legacy).unwrap();
    assert_eq!(session_file::saved_session_info_in(&state.0, &repo.0, NOW).unwrap().turns, 1);
    let mut r = persisting(&repo, &state, &[Claude]);
    assert_eq!(r.engine.resume().unwrap().turns, 1);
    r.turn("carry on", want(TaskType::Feature)).await;
    assert_eq!(r.engine.sessions.turns(), 2);
    assert_eq!(std::fs::read_dir(session_file::project_sessions_dir(&state.0, &repo.0)).map_or(0, Iterator::count), 0);
}

#[tokio::test]
async fn old_sessions_are_not_resumed_and_projects_do_not_see_each_other() {
    let (repo, other, state) = (Repo::new("persist-age", false), Repo::new("persist-other", false), StateDir::new("persist-age"));
    let mut r = persisting(&repo, &state, &[Claude]);
    r.turn("hello", want(TaskType::Feature)).await;

    let mut elsewhere = persisting(&other, &state, &[Claude]);
    assert_eq!(elsewhere.engine.resume().unwrap_err(), ResumeError::NotFound);

    let mut later = persisting(&repo, &state, &[Claude]);
    later.clock.store(NOW + 15 * 24 * 60 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(later.engine.resume().unwrap_err(), ResumeError::NotFound);
    later.clock.store(NOW + 13 * 24 * 60 * 60 * 1000, Ordering::SeqCst);
    assert!(later.engine.resume().is_ok());
}

#[tokio::test]
async fn a_garbage_saved_file_is_reported_and_a_new_session_saves_beside_it() {
    let (repo, state) = (Repo::new("persist-bad", false), StateDir::new("persist-bad"));
    let mut r = persisting(&repo, &state, &[Claude]);
    let dir = session_file::project_sessions_dir(&state.0, &repo.0);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("1-1.json"), "{ this is not json").unwrap();
    assert!(matches!(r.engine.resume(), Err(ResumeError::Corrupt(_))));
    assert_eq!(r.turn("hello", want(TaskType::Feature)).await.status, TurnStatus::Answered);
    let mut again = persisting(&repo, &state, &[Claude]);
    assert_eq!(again.engine.resume().unwrap().turns, 1, "the good session is found past the damaged file");
}

#[tokio::test]
async fn a_save_that_cannot_be_written_never_fails_the_turn_but_save_now_says_why() {
    let repo = Repo::new("persist-ro", false);
    // A regular file where the state directory should be.
    let blocker = std::env::temp_dir().join(format!("jev-turn-blocker-{}", std::process::id()));
    std::fs::write(&blocker, "x").unwrap();
    let mut r = rig_in(repo.0.clone(), &states_for(&[Claude]), OK);
    r.engine = r.engine.with_persistence(Some(blocker.join("state")));
    let report = r.turn("hello", want(TaskType::Feature)).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert!(r.engine.save_now().is_err());
    std::fs::remove_file(&blocker).ok();
}
