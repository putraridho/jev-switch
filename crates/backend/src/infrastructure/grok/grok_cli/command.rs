//! The headless `grok` command line for one turn.

use tokio::process::Command;

use super::GrokBackend;
use jev_approval::domain::mode::Mode;
use crate::{PromptArg, TurnRequest};

/// `--always-approve`: a routed turn must not get blanket tool access.
const PERMISSION_MODE: &str = "default";
/// Tools allowed without a prompt in Edit and Auto only. `Write` is an alias of
/// `Edit`; Ask and Plan receive no allow rules.
const ALLOWED_TOOLS: [&str; 2] = ["Edit", "Write"];

impl GrokBackend {
    pub(super) fn turn_command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        // `--single=` (not `-p <prompt>`) so a prompt that starts with `-` is not read as a flag.
        match self.prompts.arg(&crate::prompt_with_images(req)) {
            PromptArg::Inline(prompt) => cmd.arg(format!("--single={prompt}")),
            PromptArg::File { path, .. } => cmd.arg("--prompt-file").arg(path),
        };
        cmd.args(["--model", &req.model, "--output-format", "streaming-messages-json"])
            .args(["--no-auto-update", "--no-alt-screen", "--permission-mode", PERMISSION_MODE])
            .current_dir(&req.dir);
        if matches!(req.mode, Mode::Edit | Mode::Auto) {
            for tool in ALLOWED_TOOLS {
                cmd.args(["--allow", tool]);
            }
        }
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd
    }

}
