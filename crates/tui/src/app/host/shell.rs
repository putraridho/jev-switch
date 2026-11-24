//! `!command`: run it in a shell, keep a bounded tail of its output for the next message.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;

/// Lines of `!command` output kept for the next message.
pub const SHELL_KEEP_LINES: usize = 200;

/// Bytes of `!command` output kept for the next message.
pub const SHELL_KEEP_BYTES: usize = 20_000;

/// What a `!command` printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRun {
    pub command: String,
    /// The retained tail of stdout and stderr, with line endings normalized.
    pub output: String,
    /// Whether earlier output was omitted.
    pub truncated: bool,
    /// The exit code; `None` when killed by a signal or it could not start.
    pub code: Option<i32>,
    /// It could not be started at all; the reason.
    pub failed: Option<String>,
}

/// Run `command` with the platform's shell (`sh -c`, `cmd /C` on Windows) in `dir`, stdin closed, stderr joined to stdout; each line
/// goes to `on_line` as it is printed, and a bounded tail comes back in the result.
pub fn run_shell(command: &str, dir: &Path, on_line: &mut dyn FnMut(&str)) -> ShellRun {
    let child = jev_shared::infrastructure::platform::shell_merged(command).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return ShellRun { command: command.to_string(), output: String::new(), truncated: false, code: None, failed: Some(e.to_string()) },
    };
    // The pipe is read on its own thread: a job the command left in the background (`npm run
    // dev &`) keeps it open after `sh` exits, and the session must not wait for that job.
    let (lines, received) = std::sync::mpsc::sync_channel::<String>(1);
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            while let Some(piece) = next_piece(&mut stdout, &mut pending, SHELL_KEEP_BYTES) {
                if lines.send(String::from_utf8_lossy(&piece).into_owned()).is_err() {
                    return;
                }
            }
        });
    }
    let mut output = ShellTail::default();
    // Whether the last piece ended mid-line (a line longer than the cap comes in pieces).
    let mut unfinished = false;
    let mut take = |line: String, output: &mut ShellTail| {
        // The UI may display a long line as multiple pieces; the model tail joins them. A
        // piece that is only the end of such a line is not a line of its own.
        let text = line.trim_end_matches(['\n', '\r']);
        if !(unfinished && text.is_empty()) {
            on_line(text);
        }
        unfinished = !line.ends_with('\n');
        output.push(&line);
    };
    let status = loop {
        match received.recv_timeout(SHELL_POLL) {
            Ok(line) => take(line, &mut output),
            // The pipe closed: everything was read.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break child.wait(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // `sh` is done; take what is still on its way, then stop waiting for the pipe.
                let until = std::time::Instant::now() + SHELL_DRAIN;
                while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
                    match received.recv_timeout(left) {
                        Ok(line) => take(line, &mut output),
                        Err(_) => break,
                    }
                }
                break Ok(status);
            }
            Ok(None) => {}
            Err(e) => break Err(e),
        }
    };
    let truncated = output.cut;
    let output = output.into_text();
    match status {
        Ok(status) => ShellRun { command: command.to_string(), output, truncated, code: status.code(), failed: None },
        Err(e) => ShellRun { command: command.to_string(), output, truncated, code: None, failed: Some(e.to_string()) },
    }
}

/// The next line `reader` gives (with its `\n`), or, of a line longer than `cap` bytes (a
/// progress bar redrawn with `\r`, binary output), the next piece of at most `cap` bytes, cut
/// before a UTF-8 character that would not fit whole. `pending` holds what was read but not
/// yet given, so memory stays near `cap` however long a line is. `None` at the end.
fn next_piece(reader: &mut impl std::io::Read, pending: &mut Vec<u8>, cap: usize) -> Option<Vec<u8>> {
    let mut buf = [0u8; 8192];
    loop {
        if let Some(end) = pending.iter().take(cap).position(|&b| b == b'\n') {
            return Some(pending.drain(..=end).collect());
        }
        if pending.len() > cap {
            // `pending[cut]` exists, so a character cut there is visible: back up to its start.
            let mut cut = cap;
            while cut > 0 && pending[cut] & 0xC0 == 0x80 {
                cut -= 1;
            }
            if cut > 1 && pending[cut] == b'\n' && pending[cut - 1] == b'\r' {
                cut -= 1;
            }
            return Some(pending.drain(..if cut == 0 { cap } else { cut }).collect());
        }
        match reader.read(&mut buf) {
            Ok(0) => return (!pending.is_empty()).then(|| std::mem::take(pending)),
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return (!pending.is_empty()).then(|| std::mem::take(pending)),
        }
    }
}

/// How often a running `!command` is checked for having exited.
const SHELL_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// How long output is still taken after the command exited (its last lines in flight).
const SHELL_DRAIN: std::time::Duration = std::time::Duration::from_millis(200);

#[derive(Default)]
struct ShellTail {
    lines: VecDeque<String>,
    bytes: usize,
    cut: bool,
    unfinished: bool,
}

impl ShellTail {
    fn push(&mut self, raw: &str) {
        let continuation = self.unfinished;
        self.unfinished = !raw.ends_with('\n');
        let line = match raw.strip_suffix('\n') {
            Some(line) => line.strip_suffix('\r').unwrap_or(line),
            None => raw,
        };
        if line.len() > SHELL_KEEP_BYTES {
            let mut start = line.len() - SHELL_KEEP_BYTES;
            while !line.is_char_boundary(start) {
                start += 1;
            }
            self.lines.clear();
            self.bytes = 0;
            self.cut = true;
            self.lines.push_back(line[start..].to_string());
            self.bytes = line.len() - start;
            return;
        }
        if continuation && !self.lines.is_empty() {
            if let Some(last) = self.lines.back_mut() {
                last.push_str(line);
                self.bytes += line.len();
            }
        } else {
            self.bytes += line.len() + usize::from(!self.lines.is_empty());
            self.lines.push_back(line.to_string());
        }
        while self.lines.len() > SHELL_KEEP_LINES {
            if let Some(first) = self.lines.pop_front() {
                self.bytes -= first.len() + 1;
                self.cut = true;
            }
        }
        while self.bytes > SHELL_KEEP_BYTES {
            let excess = self.bytes - SHELL_KEEP_BYTES;
            let Some(first) = self.lines.front_mut() else { break };
            if excess > first.len() {
                self.bytes -= first.len() + 1;
                self.lines.pop_front();
            } else {
                let mut start = excess;
                while !first.is_char_boundary(start) {
                    start += 1;
                }
                *first = first[start..].to_string();
                self.bytes -= start;
            }
            self.cut = true;
        }
    }

    fn into_text(self) -> String {
        self.lines.into_iter().collect::<Vec<_>>().join("\n")
    }
}

/// `text` cut to the last [`SHELL_KEEP_LINES`] lines and [`SHELL_KEEP_BYTES`] bytes (the
/// end of a long output is usually what matters), and whether anything was cut.
pub fn keep_tail(text: &str) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let mut cut = lines.len() > SHELL_KEEP_LINES;
    let mut kept = lines[lines.len().saturating_sub(SHELL_KEEP_LINES)..].join("\n");
    if kept.len() > SHELL_KEEP_BYTES {
        let mut start = kept.len() - SHELL_KEEP_BYTES;
        while !kept.is_char_boundary(start) {
            start += 1;
        }
        kept = kept[start..].to_string();
        cut = true;
    }
    (kept, cut)
}

/// The note the next message carries about a `!command`, so the model sees what the
/// person saw.
pub fn shell_note(run: &ShellRun) -> String {
    if let Some(why) = &run.failed {
        return format!("I tried to run `{}` in the shell, but it could not start: {why}", run.command);
    }
    let status = match run.code {
        Some(0) => "it exited 0".to_string(),
        Some(code) => format!("it exited {code}"),
        None => "it was stopped by a signal".to_string(),
    };
    let cut_note = if run.truncated { " (only the end is shown)" } else { "" };
    format!("I ran `{}` in the shell; {status}. Its output{cut_note}:\n```\n{}\n```", run.command, run.output.trim_end())
}

#[cfg(test)]
mod tests;
