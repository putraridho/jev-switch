//! Machine checks: git repository, state directory, key-file permissions.

use super::{check, steps, Check, EnvFileFacts, Severity};

pub(super) fn git_check(in_repo: Option<bool>) -> Option<Check> {
    Some(match in_repo? {
        true => check("git-repo", "This folder is a git repository", Severity::Ok, "Codex and Gemini will run here; /undo and the change summary work.", Vec::new()),
        false => check(
            "git-repo",
            "This folder is not a git repository",
            Severity::Warn,
            "Codex and Gemini refuse non-git or untrusted folders, and /undo and the change summary need git.",
            steps(&["git init", "or start the router from inside a git repository"]),
        ),
    })
}

pub(super) fn state_dir_check(writable: Option<bool>) -> Option<Check> {
    Some(match writable? {
        true => check("state-dir", "State directory is writable", Severity::Ok, "Logs, cooldowns and saved sessions can be kept.", Vec::new()),
        false => check(
            "state-dir",
            "State directory is not writable",
            Severity::Warn,
            "Logs, cooldowns and saved sessions need it; without it they are lost between runs.",
            steps(&["mkdir -p ~/.local/state/jev-router", "chmod u+rwx ~/.local/state/jev-router"]),
        ),
    })
}

/// A key file anyone else can read is a leak. Left out when there is no file or
/// nothing secret in it.
pub(super) fn env_perms_check(file: Option<&EnvFileFacts>) -> Option<Check> {
    let f = file.filter(|f| f.has_keys)?;
    let path = f.path.display().to_string();
    Some(match f.mode_bits {
        Some(bits) if bits & 0o077 != 0 => check(
            "env-perms",
            format!("{path} is readable by other users"),
            Severity::Warn,
            format!("It holds keys and its mode is {:03o}; only you should be able to read it.", bits & 0o777),
            vec![format!("chmod 600 {path}")],
        ),
        _ => check("env-perms", format!("{path} is private"), Severity::Ok, "Only you can read the key file.", Vec::new()),
    })
}
