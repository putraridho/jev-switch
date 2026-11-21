//! Saved-session data, resume information and expiry rules.

use std::path::PathBuf;

use super::session::Sessions;
use crate::shared::ProviderId;

/// Longest reply kept in a saved session; the catch-up shrinks replies far below this anyway.
pub const SAVED_REPLY_CHARS: usize = 4_000;
/// A session saved to disk is ignored once it is older than this.
pub const SAVED_MAX_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// What is stored for one project: the conversation plus where routing stood.
#[derive(Debug, Clone)]
pub struct SavedSession {
    pub project: String,
    pub saved_ms: u64,
    pub sessions: Sessions,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

/// What the UI shows to offer resuming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub turns: usize,
    pub last_used_ms: u64,
    pub providers: Vec<ProviderId>,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    /// Persistence is switched off for this engine.
    Disabled,
    /// Nothing saved for this project (or it is older than 14 days, or for another project).
    NotFound,
    /// The file is not a saved session.
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("saving sessions is switched off"),
            Self::NotFound => f.write_str("no earlier saved session for this folder (the last 10 are kept, for 14 days)"),
            Self::Corrupt(m) => write!(f, "the saved session is unreadable: {m}"),
            Self::Io(m) => write!(f, "could not read the saved session: {m}"),
        }
    }
}

impl std::error::Error for ResumeError {}

impl SavedSession {
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            turns: self.sessions.turns(),
            last_used_ms: self.saved_ms,
            providers: self.sessions.providers_used(),
            current: self.current.clone(),
        }
    }

    /// Is this too old to offer at time `now_ms`?
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.saved_ms) > SAVED_MAX_AGE_MS
    }
}

/// One saved session as the `/resume` list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub info: SessionInfo,
    /// What the person first asked.
    pub title: String,
}
