//! Running hook commands: the child process, its stdin and bounded stderr, the timeout.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Event, Hooks, Outcome};

pub(super) fn clip(command: &str) -> String {
    let one_line = command.lines().next().unwrap_or("");
    if one_line.chars().count() > 50 {
        format!("{}...", one_line.chars().take(47).collect::<String>())
    } else {
        one_line.to_string()
    }
}

pub(super) enum Ran {
    Exited { code: i32, stderr: String },
    TimedOut,
    NotStarted(String),
}

/// How much of a hook's stderr is kept.
pub(super) const STDERR_LIMIT: usize = 64 * 1024;

/// The gathered stderr as text: cut at a character boundary, and marked when it was cut (the
/// reader keeps one byte past the limit to tell).
pub(super) fn kept_text(bytes: &[u8]) -> String {
    if bytes.len() <= STDERR_LIMIT {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let cut = &bytes[..STDERR_LIMIT];
    // Drop only a character the cut split (an incomplete sequence at the end), not a bad byte
    // the hook really wrote there.
    let end = match std::str::from_utf8(cut) {
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        _ => STDERR_LIMIT,
    };
    let mut text = String::from_utf8_lossy(&cut[..end]).into_owned();
    text.push_str("…(cut)");
    text
}

pub(super) fn run_command(command: &str, dir: &Path, input: &str, event: Event, tool: Option<&str>, timeout: Duration) -> Ran {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(dir)
        .env("JEV_HOOK_EVENT", event.as_str())
        .env("JEV_HOOK_TOOL", tool.unwrap_or(""))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Its own process group, as the backends' runner does: a timeout stops everything the hook
    // started, not only `sh`; Ctrl-C at jev's prompt does not kill a guard hook halfway (which
    // would read as a failure, not a refusal); and a hook cannot read the terminal under jev's
    // own readers (its stdin is the event; a tty prompt stops it until the timeout).
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran::NotStarted(e.to_string()),
    };
    // The input is written on its own thread: a hook that never reads it would otherwise block
    // the write (past a pipe's worth of bytes) before the timeout is ever looked at. A hook that
    // does not read its input closes the pipe early: not an error.
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    // Stderr is gathered as it arrives, so what was written is kept even if the pipe never closes.
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let (sent, received) = std::sync::mpsc::channel::<()>();
    if let Some(mut s) = child.stderr.take() {
        let written = written.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                let n = match std::io::Read::read(&mut s, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                // Kept to a bound: a background process may write to it for the whole session.
                if let Ok(mut w) = written.lock() {
                    let room = (STDERR_LIMIT + 1).saturating_sub(w.len());
                    w.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
            let _ = sent.send(());
        });
    } else {
        drop(sent);
    }
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                #[cfg(unix)]
                if let Ok(pid) = i32::try_from(child.id()) {
                    // SAFETY: plain `kill(2)` on a negative pid: the process group made above.
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Ran::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Ran::NotStarted(e.to_string()),
        }
    };
    // A hook that left a background process holding stderr open would keep the reader waiting
    // for as long as that process lives: take what was written so far, and go on without the rest.
    let _ = received.recv_timeout(Duration::from_millis(500));
    let stderr = written.lock().map(|w| kept_text(&w)).unwrap_or_default();
    Ran::Exited { code: status.code().unwrap_or(-1), stderr }
}

/// Run `pre_tool` hooks off the async runtime's threads; the refusal text when one refused.
pub async fn refusal_for(hooks: &Arc<Hooks>, tool: &str, summary: &str) -> Option<Outcome> {
    if hooks.is_empty() {
        return None;
    }
    let (hooks, tool, summary) = (hooks.clone(), tool.to_string(), summary.to_string());
    tokio::task::spawn_blocking(move || hooks.run(Event::PreTool, Some(&tool), Some(&summary))).await.ok()
}

/// Run hooks for an event nobody waits for: on their own thread, results dropped.
pub fn fire(hooks: &Arc<Hooks>, event: Event, tool: Option<&str>, summary: Option<&str>) {
    if hooks.is_empty() {
        return;
    }
    let (hooks, tool, summary) = (hooks.clone(), tool.map(str::to_string), summary.map(str::to_string));
    std::thread::spawn(move || {
        hooks.run(event, tool.as_deref(), summary.as_deref());
    });
}
