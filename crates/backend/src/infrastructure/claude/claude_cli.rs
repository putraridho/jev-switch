//! Claude Code backend: `claude -p --output-format stream-json --verbose`. The same
//! CLI also serves Anthropic-compatible APIs (DeepSeek) when started with extra env
//! vars, so one struct covers every provider that speaks Claude Code's stream.
//!
//! In a session ([`ClaudeBackend::in_session`], what `jev-code` uses) the CLI runs with
//! `--input-format stream-json --permission-prompt-tool stdio`: instead of denying a tool
//! call that needs approval it sends a `control_request` (`can_use_tool`) on stdout and
//! waits for a `control_response` on stdin, so `jev-code` can ask the person. Recorded
//! lines are in `tests/fixtures/claude/`. The CLI can withdraw a question it asked
//! (`{"type":"control_cancel_request","request_id":...}`, from the protocol schema) when the
//! turn is interrupted or something else settled it; no answer is sent for it then.
//! Headless uses (the gateway, workers, `ask`) and a `jev-code` run nobody can answer
//! (`jev-code -p`, piped input; see [`jev_providers::application::provider_registry::run_backends`]) keep plain `-p`,
//! where the CLI itself denies what is not allowed.


use std::path::PathBuf;

use serde_json::{json, Value};
use tokio::process::Command;

use crate::{
    classify_common, Backend, Decision, ErrorKind, Event, McpAttachment, PermissionRequest, ProviderId, TurnRequest,
};

mod command;
pub mod mcp_config;
mod permission_wire;
mod stream_events;
mod stream_input;
pub mod tool_summary;

use permission_wire::{permission_request, withdrawn_request};
use stream_events::{assistant_events, plan_of, result_event, stream_delta, user_events};

#[cfg(test)]
mod tests;

/// What the model is told about where it runs. A constant, so the text is byte-identical
/// on every turn and the prompt cache holds.
pub const SESSION_NOTE: &str = "You are running inside jev-code, a terminal router for coding models, not in the Claude Code terminal UI. \
When a tool call needs approval, jev-code asks the user in the terminal by itself: just make the call, and if it is denied, say what you need and why. \
The user manages permission rules with jev-code's /permissions command and the permission mode (ask, edit, plan, auto) with /mode or Shift+Tab. \
Never tell the user to use Claude Code interface features such as /config or plugin commands: jev-code does not have them. \
Read and change files with Read and Edit, not sed or heredocs; search with Grep or Glob if you have them, else grep or find. Keep shell commands plain: scripts are asked about every time. \
Ask the user questions in your reply, not with a tool. \
Wait for commands to finish before you answer: you cannot speak again until the user writes, so never promise to report back later.";

/// The environment variable that sets Claude Code's thinking budget in tokens.
const THINKING_ENV: &str = "MAX_THINKING_TOKENS";

pub struct ClaudeBackend {
    pub bin: String,
    pub provider: ProviderId,
    /// Extra environment for the child, e.g. `ANTHROPIC_BASE_URL` for DeepSeek.
    pub env: Vec<(String, String)>,
    pub allowed_tools: Option<String>,
    /// MCP config file passed with `--mcp-config` (the orchestration planner's delegate tool).
    pub mcp_config: Option<PathBuf>,
    /// The attached server's name, to allow its tools without a prompt.
    mcp_name: Option<String>,
    /// Run as a `jev-code` session: tool approvals are asked to the host and the model is
    /// told where it runs.
    interactive: bool,
    /// Ask for partial assistant text (`--include-partial-messages`), for the live screen.
    streaming: bool,
}

impl ClaudeBackend {
    /// Claude Code with the user's own login. Binary and permissions come from the
    /// environment so every front end (code, gateway workers) behaves the same.
    pub fn claude() -> Self {
        Self::with_profile(ProviderId::Claude, Vec::new())
    }

    /// The same CLI for another provider, pointed at its API through `env`.
    pub fn with_profile(provider: ProviderId, env: Vec<(String, String)>) -> Self {
        Self {
            bin: jev_shared::infrastructure::env_file::lookup("CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            provider,
            env,
            allowed_tools: jev_shared::infrastructure::env_file::lookup("CODE_ALLOWED_TOOLS"),
            mcp_config: None,
            mcp_name: None,
            interactive: false,
            streaming: false,
        }
    }

    /// For a `jev-code` session: the CLI asks the host before a tool that needs approval
    /// (see the module docs) instead of denying it.
    pub fn in_session(mut self) -> Self {
        self.interactive = true;
        self
    }
}

impl Backend for ClaudeBackend {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn command(&self, req: &TurnRequest) -> Command {
        self.build_command(req)
    }

    /// Stream-json input is one JSON message per line, not the bare prompt.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        self.stdin_message(req)
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn steerable(&self) -> bool {
        self.interactive
    }

    fn steer_line(&self, text: &str) -> Option<String> {
        if !self.interactive {
            return None;
        }
        self.steer_message(text)
    }

    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        let answer = match decision {
            Decision::Allow | Decision::AllowAlways => json!({ "behavior": "allow", "updatedInput": jev_approval::infrastructure::tool_input_wire::tool_input_to_json(&req.input) }),
            Decision::Deny { reason } => json!({ "behavior": "deny", "message": reason }),
        };
        let line = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": req.id, "response": answer } });
        Some(line.to_string())
    }

    /// DeepSeek runs through the same CLI, so it can have the tool attached too.
    fn supports_mcp(&self) -> bool {
        true
    }

    fn attach_mcp(&mut self, mcp: &McpAttachment) {
        self.mcp_config = Some(mcp.config_path.clone());
        self.mcp_name = Some(mcp.name.clone());
    }

    /// Claude Code reads its thinking budget from `MAX_THINKING_TOKENS`.
    fn set_effort(&mut self, effort: crate::Effort) {
        self.env.retain(|(k, _)| k != THINKING_ENV);
        self.env.push((THINKING_ENV.to_string(), effort.thinking_tokens().to_string()));
    }

    fn set_streaming(&mut self, on: bool) {
        self.streaming = on;
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("assistant") => assistant_events(&ev),
            Some("stream_event") => stream_delta(&ev),
            Some("user") => user_events(&ev),
            Some("system") if ev["subtype"] == "permission_denied" => vec![Event::Hint(format!(
                "\u{2717} denied {} \u{2014} allow it via CODE_ALLOWED_TOOLS or your Claude Code settings",
                ev["tool_name"].as_str().unwrap_or("tool")
            ))],
            Some("result") => vec![result_event(&ev)],
            Some("control_request") => permission_request(&ev).map(Event::PermissionRequest).into_iter().collect(),
            Some("control_cancel_request") => withdrawn_request(&ev).into_iter().collect(),
            Some("rate_limit_event") => plan_of(&ev).map(Event::Plan).into_iter().collect(),
            _ => vec![],
        }
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        classify_common(message).unwrap_or(ErrorKind::Other)
    }
}
