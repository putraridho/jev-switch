//! Gemini backend. Google ships two headless CLIs for Gemini models and a user may
//! have either logged in: `gemini` (Gemini CLI) and `agy` (Antigravity CLI, which
//! replaced Gemini CLI for free-tier and Google One users on 2026-06-18). Both are
//! one provider ([`ProviderId::Gemini`]), so one struct serves both and only the
//! flags and event shapes that genuinely differ branch on [`Flavor`].
//!
//! Gemini CLI uses headless stream-json. Antigravity uses ACP so tool approvals
//! reach the host instead of being silently denied by print mode. The legacy
//! headless fixtures remain as regression evidence; see `tests/fixtures/agy/README.md`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::Value;
use tokio::process::Command;

use crate::backend::{classify_common, Backend, Decision, ErrorKind, Event, PermissionRequest, ProviderId, TurnRequest};

pub mod auth;
mod command;
mod errors;
pub(super) mod stream_events;

use command::{bin_from_env, gemini_args};
use errors::{classify_text, reset_after};

#[cfg(test)]
mod tests;

/// Which of Google's two CLIs a [`GeminiBackend`] drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    GeminiCli,
    Antigravity,
}

/// What a blank answer is called, so [`classify_text`] can map it to `Empty`.
const EMPTY_ANSWER: &str = "the CLI returned an empty response";

/// What the current turn has said so far. Gemini CLI's `result` event carries no
/// answer text, only stats, so the answer must be rebuilt from the `message`
/// deltas that came before it.
#[derive(Default)]
struct Pending {
    /// Assistant text since the last tool call (the final answer once `result` arrives).
    text: String,
    session_id: Option<String>,
    /// Last `error` event, kept because an empty reply ends with a bare error `result`.
    notice: Option<String>,
}

/// A Gemini CLI or Antigravity CLI turn runner.
///
/// Holds per-turn parse state (Gemini CLI streams the answer in pieces, and
/// [`Backend::parse_line`] takes `&self`), so use one instance per turn: a second
/// turn running at the same time on the same instance would mix their text. The
/// constructors are cheap. Build with [`GeminiBackend::new`] or the `gemini_cli` /
/// `antigravity` constructors, not a struct literal.
pub struct GeminiBackend {
    pub flavor: Flavor,
    pub bin: String,
    pending: Mutex<Pending>,
    acp: Mutex<super::acp::Acp>,
    /// Variables the child needs that may live only in `~/.jev-router.env` (an API key),
    /// which the child process would not otherwise see.
    extra_env: Vec<(String, String)>,
}

impl GeminiBackend {
    pub fn new(flavor: Flavor, bin: impl Into<String>) -> Self {
        Self {
            flavor,
            bin: bin.into(),
            pending: Mutex::new(Pending::default()),
            acp: Mutex::default(),
            extra_env: Vec::new(),
        }
    }

    /// Pass these variables to the CLI (see `extra_env`).
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    /// Gemini CLI; the binary is `GEMINI_BIN` else `gemini`.
    pub fn gemini_cli() -> Self {
        Self::new(Flavor::GeminiCli, bin_from_env("GEMINI_BIN", "gemini"))
    }

    /// Antigravity CLI; the binary is `AGY_BIN` else `agy`.
    pub fn antigravity() -> Self {
        Self::new(Flavor::Antigravity, bin_from_env("AGY_BIN", "agy"))
    }

    /// A poisoned lock only means an earlier parse panicked; the state is plain
    /// text and every `init` resets it, so carrying on is safe.
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn args(&self, req: &TurnRequest) -> Vec<String> {
        match self.flavor {
            Flavor::GeminiCli => gemini_args(req),
            Flavor::Antigravity => vec!["acp".into()],
        }
    }
}

impl Backend for GeminiBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(self.args(req)).current_dir(&req.dir);
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd
    }

    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        match self.flavor {
            Flavor::GeminiCli => Some(crate::backend::prompt_with_images(req)),
            Flavor::Antigravity => Some(self.acp.lock().unwrap_or_else(PoisonError::into_inner).start(req)),
        }
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match self.flavor {
            Flavor::GeminiCli => self.gemini_events(&ev),
            Flavor::Antigravity => self.acp.lock().unwrap_or_else(PoisonError::into_inner).parse(&ev),
        }
    }

    fn interactive(&self) -> bool {
        self.flavor == Flavor::Antigravity
    }

    fn take_responses(&self) -> Vec<String> {
        self.acp.lock().unwrap_or_else(PoisonError::into_inner).drain()
    }

    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        self.acp.lock().unwrap_or_else(PoisonError::into_inner).answer(req, decision)
            .map(|line| line.trim_end_matches('\n').to_string())
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        match classify_common(message).or_else(|| classify_text(message)) {
            Some(ErrorKind::Quota { resets_at: None }) => ErrorKind::Quota { resets_at: reset_after(message) },
            Some(kind) => kind,
            None => ErrorKind::Other,
        }
    }
}
