//! The few places Unix and Windows differ for everything above: running a command line through
//! the platform's shell, stopping a process and what it started, and the home folder.

use std::process::Command;

/// A command that runs `script` through the platform's shell: `sh -c` on Unix, `cmd /C` on
/// Windows. The script is the person's (a `!` command, a hook, a check), so it is written in
/// the platform's own shell language.
pub fn shell(script: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        // `/S` strips exactly the outer quotes, so the script reaches cmd as typed.
        cmd.args(["/D", "/S", "/C"]).raw_arg(format!("\"{script}\""));
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }
}

/// [`shell`] with the script's stderr folded into its stdout, so one pipe carries both in order.
pub fn shell_merged(script: &str) -> Command {
    #[cfg(windows)]
    {
        shell(&format!("({script}) 2>&1"))
    }
    #[cfg(not(windows))]
    {
        shell(&format!("exec 2>&1\n{script}"))
    }
}

/// Stop process `pid` and everything it started. On Unix `pid` leads its own process group
/// (the caller made it with `process_group(0)`), so the group is signalled; on Windows
/// `taskkill /T` walks the process tree. Best effort: a process that is already gone is fine.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    if let Some(target) = crate::domain::platform::group_signal_target(pid) {
        // SAFETY: plain `kill(2)` on a negative pid, a process group the caller created; no
        // memory is involved.
        unsafe {
            libc::kill(target, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Windows programs find the home folder in `USERPROFILE`; everything here reads `HOME`. Call
/// once at the start of a program (before it reads settings) so `HOME` is set on Windows too.
/// A `HOME` that is already set (Git Bash, MSYS) is left alone, and nothing changes on Unix.
pub fn ensure_home() {
    #[cfg(windows)]
    if std::env::var_os("HOME").is_none_or(|h| h.is_empty()) {
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|p| !p.is_empty()) {
            std::env::set_var("HOME", profile);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_runs_a_script_and_merges_its_streams() {
        let out = shell_merged(if cfg!(windows) { "echo out& echo err 1>&2" } else { "echo out; echo err >&2" }).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
        assert!(text.contains("out") && text.contains("err"), "{text:?}");
        assert!(out.stderr.is_empty(), "stderr was folded into stdout");
        assert!(shell("exit 3").status().is_ok_and(|s| s.code() == Some(3)));
    }
}
