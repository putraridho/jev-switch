//! xAI Grok Build backend: `grok --single=<prompt> --output-format streaming-messages-json`,
//! or for a delegated worker `grok agent stdio` over ACP (see [`GrokBackend::over_acp`]).
//!
//! Written from the official docs only, never against a recorded run (see
//! `tests/fixtures/grok/README.md`). Two choices follow from the docs:
//! - Headless Grok does not read stdin, so the prompt travels in the args. A prompt
//!   too long for one argument goes through the documented `--prompt-file <PATH>`
//!   instead (user guide 14-headless-mode.md), see [`PromptFiles`].
//! - `streaming-messages-json` is used instead of `streaming-json` because only it
//!   has one line holding the whole answer (`result`). `streaming-json` sends the
//!   answer as `text` fragments and its `end` line has no text, which a stateless
//!   [`Backend::parse_line`] cannot assemble without sharing state between turns.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::process::Command;

use crate::infrastructure::gemini::acp::{Acp, Agent};
use crate::{Backend, Decision, ErrorKind, Event, PermissionRequest, PromptFiles, ProviderId, TurnRequest};

mod command;
mod errors;
pub mod key_home;
mod stream_events;
#[cfg(test)]
mod tests;


/// The variable the Grok CLI reads an xAI API key from, as an alternative to `grok login`.
pub const API_KEY_ENV: &str = "XAI_API_KEY";

pub struct GrokBackend {
    pub bin: String,
    /// Prompt files of the current turn; one instance serves one turn.
    prompts: PromptFiles,
    /// Variables the child needs that may live only in `~/.jev-router.env` (the API key),
    /// which the child process would not otherwise see.
    extra_env: Vec<(String, String)>,
    /// `Some` when the turn runs over ACP (`grok agent stdio`, see [`GrokBackend::over_acp`]):
    /// its tool questions then reach the host's approver instead of being decided by the CLI.
    acp: Option<Mutex<Acp>>,
}

impl GrokBackend {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into(), prompts: PromptFiles::default(), extra_env: Vec::new(), acp: None }
    }

    /// Run over ACP (`grok agent stdio`) instead of headless `--single`. Headless Grok decides
    /// shell commands itself and denies them with nobody to ask; over ACP each question goes to
    /// the host's approver, which is how a delegated worker gets the worker rule. A session on a
    /// terminal keeps headless Grok: over ACP every edit would be asked about (Grok has no
    /// accept-edits mode).
    pub fn over_acp(mut self) -> Self {
        self.acp = Some(Mutex::new(Acp::for_agent(Agent::Grok)));
        self
    }

    /// A poisoned lock only means an earlier parse panicked; `start` resets the state.
    fn acp(&self) -> Option<MutexGuard<'_, Acp>> {
        self.acp.as_ref().map(|m| m.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Pass these variables to the CLI (see `extra_env`).
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    /// Grok Build with the user's own login or `XAI_API_KEY`; the binary can be
    /// overridden with `GROK_BIN`.
    pub fn grok() -> Self {
        Self::new(jev_shared::infrastructure::env_file::lookup("GROK_BIN").unwrap_or_else(|| "grok".into()))
    }
}

/// `XAI_API_KEY` when it is set in the environment OR in `~/.jev-router.env`, so a key
/// kept only in that file still reaches the CLI (and the probe agrees).
pub fn auth_env(lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    lookup(API_KEY_ENV).filter(|v| !v.trim().is_empty()).map(|v| (API_KEY_ENV.to_string(), v)).into_iter().collect()
}

/// Whether Grok has ever stored state for this user (`<home>/.grok`). Lets an
/// availability probe guess "installed and used" without running the binary,
/// which is a lazy installer on some machines. `GROK_HOME` is not consulted.
pub fn session_dir_exists(home: &Path) -> bool {
    home.join(".grok").exists()
}

impl Backend for GrokBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Grok
    }

    fn command(&self, req: &TurnRequest) -> Command {
        if self.acp.is_none() {
            return self.turn_command(req);
        }
        let mut cmd = Command::new(&self.bin);
        cmd.args(["agent", "stdio"]).current_dir(&req.dir).envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd
    }

    /// Headless Grok ignores piped stdin (the prompt is in the args); over ACP stdin carries
    /// the protocol, starting with `initialize`.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        self.acp().map(|mut acp| acp.start(req))
    }

    fn cleanup(&self) {
        self.prompts.cleanup();
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        match self.acp() {
            Some(mut acp) => serde_json::from_str(line).map(|ev| acp.parse(&ev)).unwrap_or_default(),
            None => stream_events::parse_line(line),
        }
    }

    fn interactive(&self) -> bool {
        self.acp.is_some()
    }

    fn take_responses(&self) -> Vec<String> {
        self.acp().map(|mut acp| acp.drain()).unwrap_or_default()
    }

    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        self.acp()?.answer(req, decision).map(|line| line.trim_end_matches('\n').to_string())
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        errors::classify(message)
    }
}
