//! Keeping the tail of a CLI's stderr for error messages.

use tokio::io::AsyncReadExt;
use tokio::process::ChildStderr;
use tokio::task::JoinHandle;

/// How much of stderr is kept in memory, and how much of it goes in a message.
const STDERR_KEEP: usize = 8192;
pub(crate) const STDERR_SHOWN: usize = 400;

/// Drain stderr concurrently, keeping only the tail: an unread pipe fills up and
/// blocks the child, and the tail is all an error message needs.
pub(crate) fn collect_stderr(stderr: Option<ChildStderr>) -> JoinHandle<String> {
    tokio::spawn(async move {
        let mut tail: Vec<u8> = Vec::new();
        let Some(mut stderr) = stderr else { return String::new() };
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buf[..n]);
            if tail.len() > STDERR_KEEP {
                tail.drain(..tail.len() - STDERR_KEEP);
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    })
}

pub(crate) fn last_chars(s: &str, n: usize) -> String {
    let s = s.trim();
    let skip = s.chars().count().saturating_sub(n);
    s.chars().skip(skip).collect()
}
