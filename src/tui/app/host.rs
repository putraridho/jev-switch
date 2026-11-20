//! Programs on this machine that session commands hand work to: a shell for `!command`,
//! `$EDITOR` for `/edit` and `/memory`, and the clipboard for `/copy`. The choices (which
//! program, how output is cut, how a note for the model reads) are pure and tested; the
//! process calls around them are thin.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ------------------------------------------------------------------- !command

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

/// Run `command` with `sh -c` in `dir`, stdin closed, stderr joined to stdout; each line
/// goes to `on_line` as it is printed, and a bounded tail comes back in the result.
pub fn run_shell(command: &str, dir: &Path, on_line: &mut dyn FnMut(&str)) -> ShellRun {
    let script = format!("exec 2>&1\n{command}");
    let child = Command::new("sh").arg("-c").arg(script).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).spawn();
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

// --------------------------------------------------------------------- editor

/// The editor to open: `$VISUAL`, else `$EDITOR`, else `vi`, split into program and
/// arguments (`code --wait` works).
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> (String, Vec<String>) {
    let chosen = [visual, editor].into_iter().flatten().map(str::trim).find(|s| !s.is_empty()).unwrap_or("vi");
    let mut words = chosen.split_whitespace().map(String::from);
    let program = words.next().unwrap_or_else(|| "vi".to_string());
    (program, words.collect())
}

/// Open `path` in the editor and wait for it to close. The error is for the user.
pub fn edit_file(path: &Path) -> Result<(), String> {
    let (program, args) = editor_command(crate::shared::infrastructure::env_file::lookup("VISUAL").as_deref(), crate::shared::infrastructure::env_file::lookup("EDITOR").as_deref());
    let status = Command::new(&program).args(&args).arg(path).status().map_err(|e| format!("cannot start the editor `{program}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the editor `{program}` exited with {status}"))
    }
}

/// A private scratch file for composing a message, with `initial` in it.
pub fn scratch_file(state_dir: Option<&Path>, initial: &str) -> std::io::Result<PathBuf> {
    let dir = state_dir.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("message-{}.md", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&path)?.write_all(initial.as_bytes())?;
    Ok(path)
}

/// Compose a message in the editor: the text written, trimmed; `Ok(None)` when it was left
/// empty. The scratch file is removed either way.
pub fn compose(state_dir: Option<&Path>) -> Result<Option<String>, String> {
    let path = scratch_file(state_dir, "").map_err(|e| format!("cannot make a scratch file: {e}"))?;
    let edited = edit_file(&path);
    let text = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    edited?;
    let text = text.map_err(|e| format!("cannot read what was written: {e}"))?;
    let text = text.trim();
    Ok((!text.is_empty()).then(|| text.to_string()))
}

// ------------------------------------------------------------------ clipboard

/// A program that takes text on stdin and puts it on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardTool {
    WlCopy,
    Xclip,
    Xsel,
    Pbcopy,
}

impl ClipboardTool {
    fn program(self) -> &'static str {
        match self {
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
            Self::Xsel => "xsel",
            Self::Pbcopy => "pbcopy",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::WlCopy | Self::Pbcopy => &[],
            Self::Xclip => &["-selection", "clipboard"],
            Self::Xsel => &["--clipboard", "--input"],
        }
    }
}

/// The clipboard programs worth trying here, best first: Wayland's when a Wayland
/// display is set, X11's when an X display is, and macOS's.
pub fn clipboard_tools(wayland: bool, x11: bool, macos: bool) -> Vec<ClipboardTool> {
    let mut tools = Vec::new();
    if wayland {
        tools.push(ClipboardTool::WlCopy);
    }
    if x11 {
        tools.extend([ClipboardTool::Xclip, ClipboardTool::Xsel]);
    }
    if macos {
        tools.push(ClipboardTool::Pbcopy);
    }
    tools
}

/// How text reached the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    By(ClipboardTool),
    /// Through the terminal (OSC 52); it works when the terminal allows it.
    Terminal,
}

/// The OSC 52 sequence that asks the terminal to set its clipboard to `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", crate::shared::domain::base64::encode(text.as_bytes()))
}

/// Put `text` on the clipboard: the first clipboard program that works, else OSC 52 on
/// stderr when it is a terminal. The error is for the user.
pub fn copy_to_clipboard(text: &str, stderr_tty: bool) -> Result<Copied, String> {
    let has = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
    let tools = clipboard_tools(has("WAYLAND_DISPLAY"), has("DISPLAY"), cfg!(target_os = "macos"));
    for tool in tools {
        let child = Command::new(tool.program()).args(tool.args()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = child else { continue };
        let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        if wrote && child.wait().is_ok_and(|s| s.success()) {
            return Ok(Copied::By(tool));
        }
    }
    if stderr_tty {
        let mut err = std::io::stderr();
        let _ = err.write_all(osc52(text).as_bytes());
        let _ = err.flush();
        return Ok(Copied::Terminal);
    }
    Err("no clipboard program found (install wl-clipboard or xclip)".to_string())
}

impl std::fmt::Display for ClipboardTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.program())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_command_runs_in_the_project_and_reports_its_exit() {
        let dir = std::env::temp_dir();
        let mut seen = Vec::new();
        let ok = run_shell("echo out; echo err >&2", &dir, &mut |l| seen.push(l.to_string()));
        assert_eq!((ok.output.as_str(), ok.code), ("out\nerr", Some(0)));
        assert_eq!(seen, ["out", "err"], "each line is passed on as it comes");
        // A job left in the background holds the pipe open; the command still returns.
        let started = std::time::Instant::now();
        let bg = run_shell("sleep 30 & echo started", &dir, &mut |_| {});
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "waited for a background job: {:?}", started.elapsed());
        assert_eq!((bg.output.as_str(), bg.code), ("started", Some(0)));
        let bad = run_shell("exit 3", &dir, &mut |_| {});
        assert_eq!(bad.code, Some(3));
        assert!(shell_note(&bad).contains("it exited 3"));
        let note = shell_note(&ok);
        assert!(note.starts_with("I ran `echo out; echo err >&2` in the shell; it exited 0. Its output:\n```\nout\nerr\n```"), "{note}");
    }

    #[test]
    fn long_output_keeps_its_end() {
        let many: String = (0..SHELL_KEEP_LINES + 5).map(|i| format!("line {i}\n")).collect();
        let (kept, cut) = keep_tail(&many);
        assert!(cut && kept.starts_with("line 5\n") && kept.ends_with(&format!("line {}", SHELL_KEEP_LINES + 4)));
        let wide = "é".repeat(SHELL_KEEP_BYTES);
        let (kept, cut) = keep_tail(&wide);
        assert!(cut && kept.len() <= SHELL_KEEP_BYTES);
        assert_eq!(keep_tail("short"), ("short".to_string(), false));
        let (output, truncated) = keep_tail(&many);
        let note = shell_note(&ShellRun { command: "x".into(), output, truncated, code: Some(0), failed: None });
        assert!(note.contains("(only the end is shown)"));
    }

    #[test]
    fn bounded_tail_matches_full_output_policy() {
        let cases = [
            "".to_string(),
            "short\r\n\nlast\r".to_string(),
            "double\r\r\nnext".to_string(),
            (0..205).map(|index| format!("line {index}\n")).collect(),
            format!("{}\n{}\n", "é".repeat(SHELL_KEEP_BYTES), "x".repeat(400)),
            format!("{}\n{}", "a".repeat(SHELL_KEEP_BYTES - 1), "é"),
            "\n".repeat(205),
            format!("{}\n\n", "a".repeat(SHELL_KEEP_BYTES)),
            format!("{}\nend\n", "a".repeat(30_000)),
            format!("{}\r\nend\n", "a".repeat(SHELL_KEEP_BYTES - 1)),
            format!("{}\r\n", "a".repeat(SHELL_KEEP_BYTES)),
            format!("{}\nend", "é".repeat(SHELL_KEEP_BYTES)),
        ];
        for text in cases {
            let mut tail = ShellTail::default();
            let (mut reader, mut pending) = (text.as_bytes(), Vec::new());
            while let Some(piece) = next_piece(&mut reader, &mut pending, SHELL_KEEP_BYTES) {
                tail.push(std::str::from_utf8(&piece).unwrap());
                assert!(tail.lines.len() <= SHELL_KEEP_LINES);
                assert!(tail.bytes <= SHELL_KEEP_BYTES);
                assert_eq!(tail.bytes, tail.lines.iter().map(String::len).sum::<usize>() + tail.lines.len().saturating_sub(1));
            }
            let expected = keep_tail(&text);
            let cut = tail.cut;
            let output = tail.into_text();
            assert_eq!((&output, cut), (&expected.0, expected.1));
            let actual = shell_note(&ShellRun { command: "x".into(), output, truncated: cut, code: Some(0), failed: None });
            let marker = if expected.1 { " (only the end is shown)" } else { "" };
            assert_eq!(actual, format!("I ran `x` in the shell; it exited 0. Its output{marker}:\n```\n{}\n```", expected.0.trim_end()));
        }
    }

    #[test]
    fn bounded_tail_never_exceeds_its_caps() {
        let mut tail = ShellTail::default();
        for index in 0..100_000 {
            tail.push(&format!("{index}: {}\n", "é".repeat(100)));
            assert!(tail.lines.len() <= SHELL_KEEP_LINES);
            assert!(tail.bytes <= SHELL_KEEP_BYTES);
            assert_eq!(tail.bytes, tail.lines.iter().map(String::len).sum::<usize>() + tail.lines.len().saturating_sub(1));
        }
    }

    #[test]
    fn pieces_are_lines_or_bounded_cuts_on_character_starts() {
        let pieces = |text: &[u8], cap: usize| {
            let (mut reader, mut pending, mut out) = (text, Vec::new(), Vec::new());
            while let Some(piece) = next_piece(&mut reader, &mut pending, cap) {
                assert!(piece.len() <= cap.max(piece.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1)));
                out.push(String::from_utf8(piece).unwrap());
            }
            out
        };
        assert_eq!(pieces(b"a\nbc\nd", 10), ["a\n", "bc\n", "d"]);
        assert_eq!(pieces(b"abcdefgh\n", 3), ["abc", "def", "gh\n"]);
        assert_eq!(pieces(b"ab\r\n", 3), ["ab", "\r\n"]);
        // `é` is two bytes: a cut never splits it.
        assert_eq!(pieces("aéé".as_bytes(), 2), ["a", "é", "é"]);
        assert!(pieces(b"", 4).is_empty());
    }

    #[test]
    fn output_without_line_breaks_stays_bounded() {
        let run = run_shell("yes x | tr -d '\\n' | head -c 3000000", Path::new("."), &mut |line| assert!(line.len() <= SHELL_KEEP_BYTES));
        assert_eq!(run.code, Some(0));
        assert!(run.truncated && run.output.len() <= SHELL_KEEP_BYTES, "{}", run.output.len());
    }

    #[test]
    fn shell_retains_long_lines_without_inserting_newlines() {
        let run = run_shell("printf '%030000d\\nend\\n' 0 | tr 0 a", Path::new("."), &mut |_| {});
        let full = format!("{}\nend\n", "a".repeat(30_000));
        assert_eq!(run.code, Some(0));
        assert_eq!((run.output.as_str(), run.truncated), (keep_tail(&full).0.as_str(), true));
        assert!(run.output.ends_with("\nend"));
        assert!(!run.output.strip_suffix("\nend").unwrap().contains('\n'));
    }

    #[test]
    fn shell_does_not_add_an_empty_line_after_a_cap_sized_line() {
        let mut seen = Vec::new();
        let run = run_shell("printf '%020000d\\nend\\n' 0 | tr 0 a", Path::new("."), &mut |line| seen.push(line.to_string()));
        let full = format!("{}\nend\n", "a".repeat(SHELL_KEEP_BYTES));
        assert_eq!(run.code, Some(0));
        assert_eq!((run.output.as_str(), run.truncated), (keep_tail(&full).0.as_str(), true));
        assert!(!run.output.contains("\n\n"));
        // The live view shows no empty row the model's tail does not have either.
        assert!(!seen.iter().any(String::is_empty), "{:?}", seen.iter().map(String::len).collect::<Vec<_>>());
        assert_eq!(seen.last().map(String::as_str), Some("end"));
    }

    #[test]
    fn shell_streams_every_line_while_retaining_only_the_tail() {
        let command = "i=0; while [ \"$i\" -lt 1000 ]; do printf 'line %s\\n' \"$i\"; i=$((i + 1)); done";
        let mut seen = Vec::new();
        let run = run_shell(command, Path::new("."), &mut |line| seen.push(line.to_string()));
        assert_eq!(run.code, Some(0));
        assert_eq!(seen.len(), 1000);
        let full = seen.join("\n");
        let (expected, truncated) = keep_tail(&full);
        assert_eq!(run.output, expected);
        assert_eq!(run.truncated, truncated);
        assert!(shell_note(&run).contains("(only the end is shown)"));
    }

    #[test]
    fn the_editor_comes_from_visual_then_editor_then_vi() {
        assert_eq!(editor_command(Some("code --wait"), Some("nano")), ("code".to_string(), vec!["--wait".to_string()]));
        assert_eq!(editor_command(Some("  "), Some("nano")), ("nano".to_string(), vec![]));
        assert_eq!(editor_command(None, None), ("vi".to_string(), vec![]));
    }

    #[test]
    fn a_scratch_file_is_private() {
        let dir = std::env::temp_dir().join(format!("jev-host-{}", std::process::id()));
        let path = scratch_file(Some(&dir), "hi").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clipboard_programs_follow_the_display() {
        use ClipboardTool::*;
        assert_eq!(clipboard_tools(true, true, false), [WlCopy, Xclip, Xsel]);
        assert_eq!(clipboard_tools(false, false, true), [Pbcopy]);
        assert!(clipboard_tools(false, false, false).is_empty());
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }
}
