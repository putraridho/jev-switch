//! The `gemini` command line of one turn.

use crate::approval::domain::mode::Mode;
use crate::backend::TurnRequest;

/// Gemini CLI's edit-approval mode, used only for Edit and Auto turns. Ask and
/// Plan use `default`; no turn uses an approve-everything mode such as `--yolo`.
const GEMINI_APPROVAL_MODE: &str = "auto_edit";

pub(super) fn bin_from_env(var: &str, default: &str) -> String {
    crate::shared::infrastructure::env_file::lookup(var).unwrap_or_else(|| default.into())
}

/// The prompt travels on stdin (see `stdin_payload`): a piped stdin already makes
/// Gemini CLI headless, and stdin has no size limit where one argv entry does.
pub(super) fn gemini_args(req: &TurnRequest) -> Vec<String> {
    let approval_mode = match req.mode {
        Mode::Edit | Mode::Auto => GEMINI_APPROVAL_MODE,
        Mode::Ask | Mode::Plan => "default",
    };
    let mut args: Vec<String> = ["-m", &req.model, "--output-format", "stream-json", "--approval-mode", approval_mode]
        .map(String::from)
        .into();
    if let Some(id) = &req.session_id {
        args.extend(["--resume".into(), id.clone()]);
    }
    args
}
