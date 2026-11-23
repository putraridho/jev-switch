//! The `codex exec` command line of one turn: sandbox, MCP overrides, images.

use tokio::process::Command;

use super::CodexBackend;
use crate::TurnRequest;

/// The only sandboxes this backend will ever pass on. Anything else, in particular
/// `danger-full-access`, silently becomes [`DEFAULT_SANDBOX`]: a stray env value must
/// never turn into an unsandboxed agent.
const SAFE_SANDBOXES: [&str; 2] = ["read-only", "workspace-write"];
pub(super) const DEFAULT_SANDBOX: &str = "workspace-write";

/// Longest a single MCP tool call may run under Codex: one hour, enough for a worker turn.
const MCP_TOOL_TIMEOUT_SECS: u64 = 3600;

impl CodexBackend {
    /// `-c model_reasoning_effort="<level>"`, accepted by both `exec` and `exec resume`.
    pub(super) fn effort_overrides(&self) -> Vec<String> {
        self.effort.map(|e| vec!["-c".into(), format!("model_reasoning_effort=\"{}\"", e.name())]).unwrap_or_default()
    }

    /// The configured sandbox if it is one of [`SAFE_SANDBOXES`], else the default.
    pub fn sandbox_arg(&self) -> &str {
        SAFE_SANDBOXES.iter().find(|s| **s == self.sandbox).copied().unwrap_or(DEFAULT_SANDBOX)
    }

    /// New turn: `-C` and `-s` are only accepted by plain `exec`.
    pub(super) fn fresh_args(&self, cmd: &mut Command, req: &TurnRequest) {
        cmd.args(["exec", "--json", "-C"]).arg(&req.dir).args(["-s", self.sandbox_for(req), "-m", &req.model]);
        cmd.args(self.mcp_overrides(req.mode)).args(self.effort_overrides()).args(self.network_overrides(req));
        // `--image=<path>` in one argument, so the flag can never take the next one as a value.
        for image in req.images.iter().filter(|p| takes_image_flag(p)) {
            cmd.arg(format!("--image={}", image.display()));
        }
    }

    /// `-c sandbox_workspace_write.network_access=true` when `CODEX_NETWORK` is on and the turn
    /// runs in `workspace-write`. The sandbox stays: writes are still confined to the
    /// workspace, only network and local ports open. Without it `vitest`, a dev server or
    /// `gh` fail with "listen EPERM" or "access is blocked". VERIFIED with `codex sandbox`
    /// (codex 0.167.0, 2026-11-23): a listen on 127.0.0.1 and a connect to api.github.com
    /// fail by default and work with this override. Never on in `read-only`.
    pub(super) fn network_overrides(&self, req: &TurnRequest) -> Vec<String> {
        if self.network && self.sandbox_for(req) == "workspace-write" {
            vec!["-c".into(), "sandbox_workspace_write.network_access=true".into()]
        } else {
            vec![]
        }
    }

    /// The sandbox of this turn: read-only in plan mode (so `/review` and `/compact` cannot
    /// edit), else the configured one.
    pub(super) fn sandbox_for(&self, req: &TurnRequest) -> &str {
        if req.mode == jev_approval::domain::mode::Mode::Plan {
            "read-only"
        } else {
            self.sandbox_arg()
        }
    }

    /// `-c mcp_servers.<name>.command=... -c mcp_servers.<name>.args=[...]`: Codex has
    /// no per-run config file flag for MCP, but `-c` takes dotted TOML overrides (checked
    /// against `codex exec --help`). A JSON string is a valid TOML basic string, so
    /// `serde_json` does the quoting. VERIFIED against codex 0.169.0 on 2026-11-17: without
    /// `default_tools_approval_mode="approve"` a headless `codex exec` refuses every MCP
    /// call ("MCP tool call requires approval, but approval policy is never"); with it
    /// the tool runs. The approval applies to this one server only.
    pub(super) fn mcp_overrides(&self, mode: jev_approval::domain::mode::Mode) -> Vec<String> {
        let Some(m) = &self.mcp else { return vec![] };
        let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let args: Vec<String> = m.args.iter().map(|a| quote(a)).collect();
        let key = format!("mcp_servers.{}", m.name);
        let command = quote(&m.command.to_string_lossy());
        let mut overrides = vec![
            "-c".into(),
            format!("{key}.command={command}"),
            "-c".into(),
            format!("{key}.args=[{}]", args.join(",")),
            // A `delegate` call runs a whole worker turn, so codex's default per-call
            // timeout (60 s) would cancel it mid-run. The field name was checked with
            // `codex exec --strict-config` (an invented field is rejected, this one is not).
            "-c".into(),
            format!("{key}.tool_timeout_sec={MCP_TOOL_TIMEOUT_SECS}"),
        ];
        if matches!(mode, jev_approval::domain::mode::Mode::Edit | jev_approval::domain::mode::Mode::Auto) {
            overrides.extend(["-c".into(), format!("{key}.default_tools_approval_mode=\"approve\"")]);
        }
        overrides
    }

    /// Continued turn: `exec resume` has neither `-C` nor `-s`, so the directory comes
    /// from `current_dir` and the sandbox is re-asserted as a config override. The
    /// trailing `-` makes codex read the prompt from stdin.
    pub(super) fn resume_args(&self, cmd: &mut Command, req: &TurnRequest, thread_id: &str) {
        let sandbox = format!("sandbox_mode=\"{}\"", self.sandbox_for(req));
        cmd.args(["exec", "resume", "--json", "-m", &req.model, "-c", &sandbox]);
        cmd.args(self.mcp_overrides(req.mode)).args(self.effort_overrides()).args(self.network_overrides(req));
        // No `--image` here: a resumed turn names its images in the prompt (see
        // `stdin_payload`), so the thread id is never taken for an image path.
        cmd.args([thread_id, "-"]);
    }
}

/// Whether an image goes as `--image=`: Codex splits that flag's value on commas, so a path
/// with one is named in the prompt instead.
pub(super) fn takes_image_flag(path: &std::path::Path) -> bool {
    !path.to_string_lossy().contains(',')
}
