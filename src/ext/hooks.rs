//! Hooks: shell commands run on events of a turn.
//!
//! ```json
//! { "hooks": [
//!   { "event": "pre_tool",  "tool": "Bash|Write", "run": "./scripts/guard.sh", "timeout_secs": 10 },
//!   { "event": "post_tool", "run": "echo done >> /tmp/tools.log" },
//!   { "event": "turn_end",  "run": "notify-send finished" } ] }
//! ```
//!
//! read from `<project>/.jev/hooks.json` and `<config>/hooks.json`. The event is sent to the
//! command as one JSON object on stdin ([`Payload`]) and in `JEV_HOOK_EVENT` / `JEV_HOOK_TOOL`.
//! A `pre_tool` hook that exits with status 2 refuses the tool call: what it wrote on stderr is
//! what the model is told. Any other failure is a warning and the call goes on.
//!
//! A `pre_tool` hook can only refuse a call the CLI asks the host about (an edit or command
//! that its own settings allow without asking never reaches jev-code). Hooks in a project
//! run code that came with the repository, so they stay off until the person trusts the
//! project once (`/hooks trust`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::Places;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Before a tool call that needs approval.
    PreTool,
    /// After the model made a tool call.
    PostTool,
    TurnStart,
    TurnEnd,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::PreTool => "pre_tool",
            Event::PostTool => "post_tool",
            Event::TurnStart => "turn_start",
            Event::TurnEnd => "turn_end",
        }
    }
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HookDef {
    pub event: Event,
    /// Tool names this applies to, `|` between alternatives; absent or `*` means every tool.
    #[serde(default)]
    pub tool: Option<String>,
    pub run: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct HooksFile {
    #[serde(default)]
    hooks: Vec<HookDef>,
}

/// What a hook is told.
#[derive(Debug, Serialize)]
pub struct Payload<'a> {
    pub event: Event,
    pub tool: Option<&'a str>,
    /// One line about the call: the command, or the file path.
    pub summary: Option<&'a str>,
    pub project: &'a Path,
}

impl HookDef {
    fn matches(&self, event: Event, tool: Option<&str>) -> bool {
        if self.event != event {
            return false;
        }
        match (self.tool.as_deref(), tool) {
            (None | Some("*"), _) => true,
            (Some(pattern), Some(tool)) => pattern.split('|').any(|p| p.trim() == tool),
            (Some(_), None) => false,
        }
    }
}

/// What running the hooks for one event came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Set by a `pre_tool` hook that exited with status 2.
    pub refused: Option<String>,
    /// Warnings: hooks that failed or timed out.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Hooks {
    defs: Vec<HookDef>,
    dir: PathBuf,
    /// Hooks in the project's own file exist but the project is not trusted, so they were not loaded.
    pub skipped_untrusted: usize,
    /// Problems reading the files, one line each.
    pub problems: Vec<String>,
}

fn trusted_file(user: &Path) -> PathBuf {
    user.join("trusted-projects")
}

pub fn is_trusted(user: &Path, project: &Path) -> bool {
    std::fs::read_to_string(trusted_file(user)).is_ok_and(|t| t.lines().any(|l| Path::new(l.trim()) == project))
}

/// Remember that the project's hooks may run. `Ok(false)`: it already was.
pub fn trust_project(user: &Path, project: &Path) -> std::io::Result<bool> {
    if is_trusted(user, project) {
        return Ok(false);
    }
    std::fs::create_dir_all(user)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(trusted_file(user))?;
    writeln!(file, "{}", project.display())?;
    Ok(true)
}

fn read_defs(path: &Path, problems: &mut Vec<String>) -> Vec<HookDef> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    match serde_json::from_str::<HooksFile>(&text) {
        Ok(file) => file.hooks.into_iter().filter(|h| !h.run.trim().is_empty()).collect(),
        Err(e) => {
            problems.push(format!("{}: {e}", path.display()));
            Vec::new()
        }
    }
}

impl Hooks {
    pub fn none(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf(), ..Self::default() }
    }

    pub fn load(places: &Places) -> Self {
        let mut hooks = Self::none(&places.project);
        if let Some(user) = &places.user {
            let defs = read_defs(&user.join("hooks.json"), &mut hooks.problems);
            hooks.defs.extend(defs);
        }
        let project_defs = read_defs(&places.project.join(".jev/hooks.json"), &mut hooks.problems);
        let trusted = places.user.as_deref().is_some_and(|u| is_trusted(u, &places.project));
        if trusted {
            hooks.defs.extend(project_defs);
        } else {
            hooks.skipped_untrusted = project_defs.len();
        }
        hooks
    }

    pub fn defs(&self) -> &[HookDef] {
        &self.defs
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Run every hook for `event` in turn (blocking).
    pub fn run(&self, event: Event, tool: Option<&str>, summary: Option<&str>) -> Outcome {
        let payload = Payload { event, tool, summary, project: &self.dir };
        let Ok(input) = serde_json::to_string(&payload) else { return Outcome::default() };
        let mut outcome = Outcome::default();
        for def in self.defs.iter().filter(|d| d.matches(event, tool)) {
            let timeout = Duration::from_secs(def.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
            match run_command(&def.run, &self.dir, &input, event, tool, timeout) {
                Ran::Exited { code: 0, .. } => {}
                Ran::Exited { code: 2, stderr } if event == Event::PreTool => {
                    let said = stderr.trim();
                    outcome.refused = Some(if said.is_empty() { format!("refused by the hook `{}`", clip(&def.run)) } else { said.to_string() });
                    return outcome;
                }
                Ran::Exited { code, .. } => outcome.warnings.push(format!("hook `{}` exited with status {code}", clip(&def.run))),
                Ran::TimedOut => outcome.warnings.push(format!("hook `{}` took longer than {}s and was stopped", clip(&def.run), timeout.as_secs())),
                Ran::NotStarted(why) => outcome.warnings.push(format!("hook `{}` did not start: {why}", clip(&def.run))),
            }
        }
        outcome
    }
}

fn clip(command: &str) -> String {
    let one_line = command.lines().next().unwrap_or("");
    if one_line.chars().count() > 50 {
        format!("{}...", one_line.chars().take(47).collect::<String>())
    } else {
        one_line.to_string()
    }
}

enum Ran {
    Exited { code: i32, stderr: String },
    TimedOut,
    NotStarted(String),
}

/// How much of a hook's stderr is kept.
const STDERR_LIMIT: usize = 64 * 1024;

fn run_command(command: &str, dir: &Path, input: &str, event: Event, tool: Option<&str>, timeout: Duration) -> Ran {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(dir)
        .env("JEV_HOOK_EVENT", event.as_str())
        .env("JEV_HOOK_TOOL", tool.unwrap_or(""))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Its own process group, so a timeout stops what the hook started, not only `sh`.
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
                    let room = STDERR_LIMIT.saturating_sub(w.len());
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
    let stderr = written.lock().map(|w| String::from_utf8_lossy(&w).into_owned()).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn hooks_with(dir: &Path, json: &str) -> Hooks {
        let user = dir.join("cfg");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("hooks.json"), json).unwrap();
        Hooks::load(&Places { project: dir.to_path_buf(), user: Some(user) })
    }

    #[test]
    fn a_status_2_pre_tool_hook_refuses_with_what_it_said() {
        let dir = temp("refuse");
        let h = hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","tool":"Bash","run":"echo no rm please >&2; exit 2"}]}"#);
        let out = h.run(Event::PreTool, Some("Bash"), Some("rm -rf /"));
        assert_eq!(out.refused.as_deref(), Some("no rm please"));
        // The matcher keeps other tools out of it.
        assert_eq!(h.run(Event::PreTool, Some("Read"), Some("a.txt")), Outcome::default());
        assert_eq!(h.run(Event::PostTool, Some("Bash"), None), Outcome::default());
    }

    #[test]
    fn the_event_arrives_as_json_on_stdin_and_in_the_environment() {
        let dir = temp("payload");
        let run = format!("cat > {0}/stdin.json; echo \"$JEV_HOOK_EVENT:$JEV_HOOK_TOOL\" > {0}/env.txt", dir.display());
        let h = hooks_with(&dir, &serde_json::json!({"hooks":[{"event":"post_tool","run":run}]}).to_string());
        assert_eq!(h.run(Event::PostTool, Some("Edit"), Some("src/a.rs")), Outcome::default());
        let stdin = std::fs::read_to_string(dir.join("stdin.json")).unwrap();
        assert!(stdin.contains(r#""event":"post_tool""#) && stdin.contains(r#""tool":"Edit""#) && stdin.contains("src/a.rs"), "{stdin}");
        assert_eq!(std::fs::read_to_string(dir.join("env.txt")).unwrap().trim(), "post_tool:Edit");
    }

    #[test]
    fn failures_and_timeouts_warn_but_do_not_refuse() {
        let dir = temp("warn");
        let h = hooks_with(&dir, r#"{"hooks":[{"event":"turn_end","run":"exit 3"},{"event":"turn_end","run":"sleep 5","timeout_secs":0},{"event":"post_tool","run":"exit 2"}]}"#);
        let out = h.run(Event::TurnEnd, None, None);
        assert_eq!(out.refused, None);
        assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("status 3") && out.warnings[1].contains("was stopped"));
        // Status 2 only refuses before a tool call.
        let post = h.run(Event::PostTool, Some("Bash"), None);
        assert_eq!((post.refused, post.warnings.len()), (None, 1));
    }

    #[test]
    fn a_hook_that_leaves_a_background_process_does_not_hold_the_turn() {
        let dir = temp("background");
        let h = hooks_with(&dir, r#"{"hooks":[{"event":"turn_end","run":"sleep 5 &"}]}"#);
        let started = Instant::now();
        let out = h.run(Event::TurnEnd, None, None);
        assert_eq!(out, Outcome::default());
        assert!(started.elapsed() < Duration::from_secs(3), "waited {:?} for a background process", started.elapsed());
    }

    #[test]
    fn a_refusal_keeps_its_reason_when_a_background_process_holds_stderr() {
        let dir = temp("background-refusal");
        let h = hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","run":"echo no rm please >&2; sleep 5 & exit 2"}]}"#);
        let started = Instant::now();
        let out = h.run(Event::PreTool, Some("Bash"), Some("rm -rf /"));
        assert!(out.refused.as_deref().is_some_and(|r| r.contains("no rm please")), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(3), "waited {:?}", started.elapsed());
    }

    #[test]
    fn a_hook_that_never_reads_a_large_input_still_times_out() {
        let dir = temp("unread-input");
        let input = "x".repeat(1 << 20);
        let started = Instant::now();
        let ran = run_command("sleep 5", &dir, &input, Event::PostTool, None, Duration::from_millis(300));
        assert!(matches!(ran, Ran::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(3), "waited {:?} on the input pipe", started.elapsed());
    }

    #[test]
    fn project_hooks_wait_for_trust() {
        let dir = temp("trust");
        let (project, user) = (dir.join("proj"), dir.join("cfg"));
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(project.join(".jev/hooks.json"), r#"{"hooks":[{"event":"turn_end","run":"true"}]}"#).unwrap();
        let places = Places { project: project.clone(), user: Some(user.clone()) };
        let before = Hooks::load(&places);
        assert_eq!((before.defs().len(), before.skipped_untrusted), (0, 1));
        assert!(trust_project(&user, &project).unwrap());
        assert!(!trust_project(&user, &project).unwrap(), "already trusted");
        let after = Hooks::load(&places);
        assert_eq!((after.defs().len(), after.skipped_untrusted), (1, 0));
    }

    #[test]
    fn a_broken_file_is_reported_not_fatal() {
        let dir = temp("broken");
        let h = hooks_with(&dir, "{ nope");
        assert!(h.is_empty() && h.problems.len() == 1 && h.problems[0].contains("hooks.json"), "{:?}", h.problems);
        let missing = Hooks::load(&Places { project: dir.clone(), user: None });
        assert!(missing.is_empty() && missing.problems.is_empty());
    }

    #[tokio::test]
    async fn the_async_helper_returns_the_refusal() {
        let dir = temp("async");
        let h = Arc::new(hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","run":"exit 2"}]}"#));
        let out = refusal_for(&h, "Bash", "ls").await.unwrap();
        assert!(out.refused.unwrap().contains("refused by the hook"));
        assert!(refusal_for(&Arc::new(Hooks::none(&dir)), "Bash", "ls").await.is_none());
    }
}
