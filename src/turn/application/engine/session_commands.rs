//! `/new`, saving, `/resume` and `/providers`.

use std::path::Path;

use crate::session::infrastructure::session_file;
use crate::session::domain::saved_session::{ResumeError, SavedSession, SessionEntry, SessionInfo};
use crate::routing::domain::decide::RoutingState;
use crate::routing::domain::sticky::Sticky;
use super::Engine;

impl Engine {
    /// `/new`: forget the conversation and every provider session. The pin, the
    /// routing switch and what is known about providers stay.
    pub fn new_session(&mut self) {
        self.sessions.reset();
        self.routing = RoutingState::default();
        self.sticky = Sticky::default();
        self.previous = None;
        self.last_context_tokens = 0;
        self.last_changes = None;
        self.change_history.clear();
        self.runtime.forget_unusable_here();
        // The saved file stays (it can be resumed); the next save starts a new one.
        self.session_file = None;
    }

    /// Point a test engine at a state folder after it was built.
    #[cfg(test)]
    pub(crate) fn set_state_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.settings = dir.clone();
        self.persist = dir;
    }

    /// The folder sessions are saved under; `None` when persistence is off.
    pub fn state_dir(&self) -> Option<&Path> {
        self.persist.as_deref()
    }

    /// The file this session saves to; `None` until its first save, when persistence is
    /// off, and after `/new`.
    pub fn session_path(&self) -> Option<&Path> {
        self.session_file.as_deref()
    }

    /// Save the session now and say why not on failure. A session with no turns is not
    /// written. Each session has its own file (see [`session_file::new_session_file`]), so
    /// saving never touches an earlier session; the oldest beyond ten are deleted.
    /// Turns save themselves best-effort; the binary calls this once more on exit.
    pub fn save_now(&mut self) -> std::io::Result<()> {
        let Some(state) = self.persist.clone() else { return Ok(()) };
        if self.sessions.turns() == 0 {
            return Ok(());
        }
        let saved_ms = (self.now)();
        let path = match &self.session_file {
            Some(p) => p.clone(),
            None => session_file::new_session_file(&state, &self.dir, saved_ms),
        };
        let saved = SavedSession {
            project: session_file::project_key(&self.dir),
            saved_ms,
            sessions: self.sessions.clone(),
            current: self.routing.current.clone(),
        };
        crate::shared::application::host::write_private(&path, &saved.to_json().to_string())?;
        session_file::prune(&state, &self.dir, &path);
        self.session_file = Some(path);
        Ok(())
    }

    /// A failed save never fails a turn.
    pub(super) fn save_quietly(&mut self) {
        let _ = self.save_now();
    }

    /// Load the most recent saved session of this folder other than the one running: the
    /// conversation, each provider's CLI session, and where routing stood. Replaces the
    /// current conversation, and from then on saves into the file it came from. Asking
    /// again steps back to the one before.
    pub fn resume(&mut self) -> Result<SessionInfo, ResumeError> {
        let state = self.persist.as_ref().ok_or(ResumeError::Disabled)?;
        let (path, saved) = session_file::load_latest(state, &self.dir, (self.now)(), self.session_file.as_deref())?;
        Ok(self.adopt(path, saved))
    }

    /// The saved sessions of this folder that `/resume` can pick up, newest first, without
    /// the one that is running.
    pub fn resumable(&self) -> Result<Vec<SessionEntry>, ResumeError> {
        let state = self.persist.as_ref().ok_or(ResumeError::Disabled)?;
        Ok(session_file::list_sessions(state, &self.dir, (self.now)(), self.session_file.as_deref()))
    }

    /// Resume the session saved in `path`, one of [`Engine::resumable`].
    pub fn resume_from(&mut self, path: &std::path::Path) -> Result<SessionInfo, ResumeError> {
        let saved = session_file::load_path(&self.dir, path)?;
        Ok(self.adopt(path.to_path_buf(), saved))
    }

    pub(super) fn adopt(&mut self, path: std::path::PathBuf, saved: SavedSession) -> SessionInfo {
        self.session_file = Some(path);
        let info = saved.info();
        self.previous = saved.sessions.last_answered().map(|(p, r)| (p.to_string(), r.to_string()));
        self.sessions = saved.sessions;
        self.routing = RoutingState::default();
        self.routing.current = saved.current;
        self.last_context_tokens = 0;
        self.last_changes = None;
        self.change_history.clear();
        info
    }

    /// The text of `/providers`.
    pub fn providers_text(&self) -> String {
        self.runtime.render((self.now)())
    }

    /// The compact view printed at startup and after `/providers refresh`.
    pub fn summary_text(&self) -> String {
        self.runtime.render_summary((self.now)())
    }

    /// `/providers refresh`: probe again.
    pub async fn refresh_providers(&mut self) {
        self.runtime.refresh().await;
    }
}
