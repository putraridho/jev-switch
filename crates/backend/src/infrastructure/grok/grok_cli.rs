//! xAI Grok Build backend: `grok --single=<prompt> --output-format streaming-messages-json`.
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

use tokio::process::Command;

use crate::{Backend, ErrorKind, Event, PromptFiles, ProviderId, TurnRequest};

mod command;
mod errors;
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
}

impl GrokBackend {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into(), prompts: PromptFiles::default(), extra_env: Vec::new() }
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
        self.turn_command(req)
    }

    /// Headless Grok ignores piped stdin; the prompt is in the args.
    fn stdin_payload(&self, _req: &TurnRequest) -> Option<String> {
        None
    }

    fn cleanup(&self) {
        self.prompts.cleanup();
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        stream_events::parse_line(line)
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        errors::classify(message)
    }
}
