//! The `claude` command line of one turn.

use tokio::process::Command;

use super::{ClaudeBackend, SESSION_NOTE};
use crate::approval::domain::rule_policy;
use super::stream_input::sends_image_blocks;
use crate::backend::TurnRequest;

/// Turns off Claude Code's background shells and monitors (see [`ClaudeBackend::build_command`]).
pub(super) const NO_BACKGROUND_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS";

impl ClaudeBackend {
    /// The user's tools plus the attached server when the mode permits auto-approval.
    fn allowed_tools_arg(&self, mode: crate::approval::domain::mode::Mode) -> Option<String> {
        let server = self.mcp_name.as_ref().filter(|_| self.mcp_config.is_some() && matches!(mode, crate::approval::domain::mode::Mode::Edit | crate::approval::domain::mode::Mode::Auto)).map(|n| format!("mcp__{n}"));
        match (&self.allowed_tools, server) {
            (Some(a), Some(s)) => Some(format!("{a} {s}")),
            (Some(a), None) => Some(a.clone()),
            (None, s) => s,
        }
    }

    pub(super) fn build_command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-p", "--model", &req.model, "--output-format", "stream-json", "--verbose"])
            .current_dir(&req.dir)
            // A turn is one process: background shells and monitors would outlive its result,
            // get killed when the process is reaped, and start turns of their own after stdin
            // is closed, where every tool approval fails with "Stream closed".
            .env(NO_BACKGROUND_ENV, "1")
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if self.streaming {
            cmd.arg("--include-partial-messages");
        }
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        let sources = setting_sources(!rule_policy::local_settings_tracked(&req.dir), req.trust_project_settings);
        cmd.args(["--setting-sources", sources, "--permission-mode", req.mode.claude_flag()]);
        if self.interactive || sends_image_blocks(self.provider, req) {
            cmd.args(["--input-format", "stream-json"]);
        }
        if self.interactive {
            // The CLI echoes each message sent on stdin once the model has it: how a message
            // typed during the turn is known to have been taken (see `steer_line`).
            cmd.arg("--replay-user-messages");
            // AskUserQuestion needs Claude Code's own screen: here it would reach the person as
            // raw JSON to approve, with no way to answer it.
            cmd.args(["--permission-prompt-tool", "stdio", "--disallowedTools", "AskUserQuestion", "--append-system-prompt", SESSION_NOTE]);
        }
        if req.mode != crate::approval::domain::mode::Mode::Plan {
            if let Some(config) = &self.mcp_config {
                cmd.arg("--mcp-config").arg(config);
            }
        }
        if let Some(allowed) = self.allowed_tools_arg(req.mode) {
            cmd.args(["--allowedTools", &allowed]);
        }
        cmd
    }
}

/// Claude settings sources permitted for this project.
pub(super) fn setting_sources(local_trusted: bool, project_trusted: bool) -> &'static str {
    match (local_trusted, project_trusted) {
        (false, false) => "user",
        (true, false) => "user,local",
        (false, true) => "user,project",
        (true, true) => "user,project,local",
    }
}
