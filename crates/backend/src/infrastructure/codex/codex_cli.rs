//! Codex backend: `codex exec --json`. One JSON event per stdout line; a turn is
//! `thread.started`, any number of `item.*` events, then `turn.completed` or
//! `turn.failed`. Unlike Claude's `result` line, `turn.completed` carries neither the
//! answer nor the thread id, so the backend remembers both while the turn streams.
//!
//! Availability: with a missing or expired login codex does not fail fast, it retries
//! the 401 for about 35 seconds. An availability probe must therefore ask
//! `codex login status` instead of discovering this by running a turn.


use std::sync::{Mutex, MutexGuard};

use serde_json::Value;
use tokio::process::Command;

use crate::{Backend, ErrorKind, Event, Finish, McpAttachment, ProviderId, TurnRequest};

mod command;
mod errors;
pub mod models;
mod stream_events;

use command::{takes_image_flag, DEFAULT_SANDBOX};
use stream_events::{failure_message, started_events, usage_of};

#[cfg(test)]
mod tests;

/// What `turn.completed` needs from earlier lines of the same turn.
#[derive(Default)]
struct TurnState {
    thread_id: Option<String>,
    /// The last `agent_message` text: codex's final answer.
    answer: String,
}

pub struct CodexBackend {
    pub bin: String,
    /// `read-only` or `workspace-write`; see [`CodexBackend::sandbox_arg`].
    pub sandbox: String,
    /// Interior mutability because [`Backend::parse_line`] takes `&self`. It is reset by
    /// every `thread.started`, so use one backend per turn: two turns streaming through
    /// the same instance at once would mix their answers.
    state: Mutex<TurnState>,
    /// The orchestration planner's delegate tool, passed as `-c` overrides.
    mcp: Option<McpAttachment>,
    /// `model_reasoning_effort`, when the person set one.
    effort: Option<crate::Effort>,
    /// Codex's `sessions` folder, where each thread's rollout records the size of its last
    /// request (see `codex_context`). `None` leaves the context size at the turn's total.
    sessions_dir: Option<std::path::PathBuf>,
}

impl CodexBackend {
    /// Codex with the user's own ChatGPT login. Binary and sandbox come from the
    /// environment so every front end (code, gateway workers) behaves the same.
    pub fn codex() -> Self {
        Self::new(
            jev_shared::infrastructure::env_file::lookup("CODEX_BIN").unwrap_or_else(|| "codex".into()),
            jev_shared::infrastructure::env_file::lookup("CODEX_SANDBOX").unwrap_or_else(|| DEFAULT_SANDBOX.into()),
        )
        .with_sessions_dir(super::codex_plan::codex_home().map(|h| h.join("sessions")))
    }

    /// Explicit binary and sandbox, for tests and callers that do not read the env.
    pub fn new(bin: impl Into<String>, sandbox: impl Into<String>) -> Self {
        Self { bin: bin.into(), sandbox: sandbox.into(), state: Mutex::new(TurnState::default()), mcp: None, effort: None, sessions_dir: None }
    }

    /// Where to find thread rollouts, for the real context size of a turn.
    pub fn with_sessions_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.sessions_dir = dir;
        self
    }

    /// The size of the thread's last request, when its rollout records one.
    fn last_request_tokens(&self, thread_id: Option<&str>) -> Option<u64> {
        super::codex_context::context_tokens_in(self.sessions_dir.as_deref()?, thread_id?)
    }


    /// A poisoned lock only means another thread panicked mid-update; the state is
    /// plain data and the next `thread.started` resets it, so keep going.
    fn state(&self) -> MutexGuard<'_, TurnState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Backend for CodexBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        match &req.session_id {
            Some(id) => self.resume_args(&mut cmd, req, id),
            None => self.fresh_args(&mut cmd, req),
        }
        cmd.current_dir(&req.dir);
        cmd
    }

    fn supports_mcp(&self) -> bool {
        true
    }

    /// The prompt, naming the images that could not go as `--image=` (all of them on a
    /// resumed turn) so Codex opens them by path.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        let listed: Vec<std::path::PathBuf> =
            req.images.iter().filter(|p| req.session_id.is_some() || !takes_image_flag(p)).cloned().collect();
        Some(crate::prompt_with_images(&TurnRequest { images: listed, ..req.clone() }))
    }

    fn attach_mcp(&mut self, mcp: &McpAttachment) {
        self.mcp = Some(mcp.clone());
    }

    fn set_effort(&mut self, effort: crate::Effort) {
        self.effort = Some(effort);
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("thread.started") => {
                *self.state() = TurnState { thread_id: ev["thread_id"].as_str().map(String::from), answer: String::new() };
                vec![]
            }
            Some("item.started") => started_events(&ev["item"]),
            Some("item.completed") => self.completed_events(&ev["item"]),
            Some("turn.completed") => {
                let (text, session_id) = {
                    let state = self.state();
                    (state.answer.clone(), state.thread_id.clone())
                };
                let last_request = self.last_request_tokens(session_id.as_deref());
                vec![Event::Finished(Finish { text, session_id, usage: usage_of(&ev["usage"], last_request) })]
            }
            // Top-level `error` lines are retry chatter ("Reconnecting... 2/5"); only
            // `turn.failed` means codex gave up.
            Some("turn.failed") => vec![Event::Failed { message: failure_message(&ev) }],
            _ => vec![],
        }
    }


    fn classify_error(&self, message: &str) -> ErrorKind {
        errors::classify_error(message)
    }
}
