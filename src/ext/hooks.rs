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
//! project's current hooks file (`/hooks trust`). Trust is bound to the file's contents:
//! any change to it turns the project's hooks off again until it is trusted anew. The same
//! trust (by path) also lets Claude Code load the project's own `.claude/settings.json`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{sha256, Places};

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
    project_start: usize,
    project_hash: Option<String>,
    untrusted_defs: Vec<HookDef>,
    /// Hooks in the project's own file exist but the project is not trusted, so they were not loaded.
    pub skipped_untrusted: usize,
    /// Problems reading the files, one line each.
    pub problems: Vec<String>,
}

fn trusted_file(user: &Path) -> PathBuf {
    user.join("trusted-projects")
}

pub fn is_trusted(user: &Path, project: &Path) -> bool {
    std::fs::read_to_string(trusted_file(user)).is_ok_and(|t| t.lines().any(|line| trusted_path(line) == project))
}

fn trusted_path(line: &str) -> &Path {
    Path::new(line.split_once('\t').map_or_else(|| line.trim(), |(path, _)| path))
}

/// A trust line is `path`, `hooks hash` and `settings hash`, tab separated. Older lines have
/// fewer columns: the path alone (the first versions) or the path and hooks hash.
fn columns(line: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut parts = line.splitn(3, '\t');
    (parts.next().unwrap_or(""), parts.next(), parts.next())
}

fn has_hash(entries: &str, project: &Path, hash: &str) -> bool {
    entries.lines().any(|line| {
        let (path, saved, _) = columns(line);
        Path::new(path) == project && saved == Some(hash)
    })
}

/// Hash of the project's `.claude/settings.json` (of nothing when there is none, which no
/// file added later can match). That file can define hooks, `env` and helper commands that
/// Claude Code runs, so it is trusted only as it was when `/hooks trust` was run.
fn settings_hash(project: &Path) -> String {
    sha256::hex(&std::fs::read(project.join(".claude/settings.json")).unwrap_or_default())
}

/// Whether the project's `.claude/settings.json` is the one that was trusted.
pub fn claude_settings_trusted(user: &Path, project: &Path) -> bool {
    let current = settings_hash(project);
    std::fs::read_to_string(trusted_file(user)).is_ok_and(|entries| {
        entries.lines().any(|line| {
            let (path, _, saved) = columns(line);
            Path::new(path) == project && saved == Some(current.as_str())
        })
    })
}

/// Remember the current project's hooks. `Ok(false)`: they were already trusted.
pub fn trust_project(user: &Path, project: &Path) -> std::io::Result<bool> {
    trust_project_with_preview(user, project).map(|(newly, _)| newly)
}

/// Remember the current hooks and return the commands that were approved.
pub fn trust_project_with_preview(user: &Path, project: &Path) -> std::io::Result<(bool, Vec<HookDef>)> {
    let path = project.to_str().filter(|path| !path.contains(['\t', '\n', '\r']))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "project path cannot be stored"))?;
    let hooks_path = project.join(".jev/hooks.json");
    // No hooks file is fine: the project can still be trusted (for its Claude settings).
    // The hash of nothing then matches no file a later change could add.
    let bytes = match std::fs::read(&hooks_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let hash = sha256::hex(&bytes);
    let settings = settings_hash(project);
    let defs = parse_defs(&hooks_path, &bytes, &mut Vec::new());
    let entries = match std::fs::read_to_string(trusted_file(user)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let matching: Vec<&str> = entries.lines().filter(|line| trusted_path(line) == project).collect();
    if matching.len() == 1 && has_hash(matching[0], project, &hash) && columns(matching[0]).2 == Some(settings.as_str()) {
        return Ok((false, defs));
    }
    std::fs::create_dir_all(user)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(trusted_file(user))?;
    for line in entries.lines().filter(|line| trusted_path(line) != project) {
        writeln!(file, "{line}")?;
    }
    writeln!(file, "{path}\t{hash}\t{settings}")?;
    Ok((true, defs))
}

fn read_defs(path: &Path, problems: &mut Vec<String>) -> Vec<HookDef> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    parse_defs(path, &bytes, problems)
}

fn parse_defs(path: &Path, bytes: &[u8], problems: &mut Vec<String>) -> Vec<HookDef> {
    match serde_json::from_slice::<HooksFile>(bytes) {
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
        hooks.project_start = hooks.defs.len();
        let path = places.project.join(".jev/hooks.json");
        if let Ok(bytes) = std::fs::read(&path) {
            let project_defs = parse_defs(&path, &bytes, &mut hooks.problems);
            let hash = sha256::hex(&bytes);
            let trusted = places.user.as_deref().and_then(|user| std::fs::read_to_string(trusted_file(user)).ok())
                .is_some_and(|entries| has_hash(&entries, &places.project, &hash));
            if trusted {
                hooks.project_hash = Some(hash);
                hooks.defs.extend(project_defs);
            } else {
                hooks.skipped_untrusted = project_defs.len();
                hooks.untrusted_defs = project_defs;
                if places.user.as_deref().is_some_and(|user| is_trusted(user, &places.project)) {
                    hooks.problems.push("project hooks changed since you trusted them; review with /hooks and run /hooks trust again".into());
                }
            }
        }
        hooks
    }

    pub fn defs(&self) -> &[HookDef] {
        &self.defs
    }

    pub fn untrusted_defs(&self) -> &[HookDef] {
        &self.untrusted_defs
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Run every hook for `event` in turn (blocking).
    pub fn run(&self, event: Event, tool: Option<&str>, summary: Option<&str>) -> Outcome {
        let payload = Payload { event, tool, summary, project: &self.dir };
        let Ok(input) = serde_json::to_string(&payload) else { return Outcome::default() };
        let mut outcome = Outcome::default();
        for (index, def) in self.defs.iter().enumerate().filter(|(_, d)| d.matches(event, tool)) {
            if index >= self.project_start && !self.project_hash.as_ref().is_some_and(|hash| {
                std::fs::read(self.dir.join(".jev/hooks.json")).is_ok_and(|bytes| sha256::hex(&bytes) == *hash)
            }) {
                outcome.warnings.push("project hooks changed since you trusted them; review with /hooks and run /hooks trust again".into());
                break;
            }
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

/// The gathered stderr as text: cut at a character boundary, and marked when it was cut (the
/// reader keeps one byte past the limit to tell).
fn kept_text(bytes: &[u8]) -> String {
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
    fn long_stderr_is_cut_at_a_character_and_marked() {
        assert_eq!(kept_text(b"short"), "short");
        // One ASCII byte first, so the limit (even) falls inside a two-byte `é`.
        let mut bytes = format!("x{}", "é".repeat(STDERR_LIMIT)).into_bytes();
        bytes.truncate(STDERR_LIMIT + 1);
        let kept = kept_text(&bytes);
        assert!(kept.ends_with("é…(cut)") && !kept.contains('\u{FFFD}'));
        // A bad byte the hook wrote right at the limit is kept (shown as U+FFFD).
        let mut bad = vec![b'a'; STDERR_LIMIT + 1];
        bad[STDERR_LIMIT - 1] = 0xFF;
        assert!(kept_text(&bad).ends_with("\u{FFFD}…(cut)"));
    }

    #[test]
    fn a_timeout_stops_what_the_hook_started() {
        let dir = temp("tree");
        let marker = dir.join("survived");
        let cmd = format!("(sleep 1; touch {}) & sleep 5", marker.display());
        let ran = run_command(&cmd, &dir, "", Event::TurnEnd, None, Duration::from_millis(200));
        assert!(matches!(ran, Ran::TimedOut));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "a process the hook started outlived its timeout");
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
    fn changed_hooks_wait_for_retrust_even_in_a_loaded_session() {
        let dir = temp("changed-trust");
        let (project, user) = (dir.join("proj"), dir.join("cfg"));
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        let path = project.join(".jev/hooks.json");
        std::fs::write(&path, r#"{"hooks":[{"event":"turn_end","run":"touch old"}]}"#).unwrap();
        let places = Places { project: project.clone(), user: Some(user.clone()) };
        assert!(trust_project(&user, &project).unwrap());
        let loaded = Hooks::load(&places);
        std::fs::write(&path, r#"{"hooks":[{"event":"turn_end","run":"touch new"}]}"#).unwrap();
        assert!(is_trusted(&user, &project));
        assert!(!loaded.run(Event::TurnEnd, None, None).warnings.is_empty());
        assert!(!project.join("old").exists());
        let changed = Hooks::load(&places);
        assert_eq!((changed.defs().len(), changed.skipped_untrusted), (0, 1));
        assert!(changed.problems.iter().any(|problem| problem.contains("changed since you trusted")));
        assert_eq!(changed.untrusted_defs()[0].run, "touch new");
        assert!(trust_project(&user, &project).unwrap());
        let entries = std::fs::read_to_string(trusted_file(&user)).unwrap();
        assert_eq!(entries.lines().count(), 1);
        let retrusted = Hooks::load(&places);
        assert_eq!((retrusted.defs().len(), retrusted.skipped_untrusted), (1, 0));
        assert_eq!(retrusted.run(Event::TurnEnd, None, None), Outcome::default());
        assert!(project.join("new").exists());
    }

    #[test]
    fn project_claude_settings_are_trusted_only_as_they_were() {
        let dir = temp("settings-trust");
        let (project, user) = (dir.join("proj"), dir.join("cfg"));
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        let settings = project.join(".claude/settings.json");
        std::fs::write(&settings, r#"{"permissions":{}}"#).unwrap();
        assert!(!claude_settings_trusted(&user, &project), "not trusted before /hooks trust");
        assert!(trust_project(&user, &project).unwrap());
        assert!(claude_settings_trusted(&user, &project));
        assert!(!trust_project(&user, &project).unwrap(), "nothing changed");
        std::fs::write(&settings, r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"curl x"}]}]}}"#).unwrap();
        assert!(is_trusted(&user, &project), "the path stays remembered");
        assert!(!claude_settings_trusted(&user, &project), "a changed settings file waits for retrust");
        assert!(trust_project(&user, &project).unwrap());
        assert!(claude_settings_trusted(&user, &project));
        assert_eq!(std::fs::read_to_string(trusted_file(&user)).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_settings_file_added_after_trust_is_not_trusted() {
        let dir = temp("settings-added");
        let (project, user) = (dir.join("proj"), dir.join("cfg"));
        std::fs::create_dir_all(&project).unwrap();
        assert!(trust_project(&user, &project).unwrap());
        assert!(claude_settings_trusted(&user, &project));
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        std::fs::write(project.join(".claude/settings.json"), r#"{"env":{"X":"1"}}"#).unwrap();
        assert!(!claude_settings_trusted(&user, &project));
    }

    #[test]
    fn legacy_trust_only_trusts_the_path_until_renewed() {
        let dir = temp("legacy-trust");
        let (project, other, user) = (dir.join("proj"), dir.join("other"), dir.join("cfg"));
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(project.join(".jev/hooks.json"), r#"{"hooks":[{"event":"turn_start","run":"true"}]}"#).unwrap();
        std::fs::write(trusted_file(&user), format!("{}\n{}\n", project.display(), other.display())).unwrap();
        let places = Places { project: project.clone(), user: Some(user.clone()) };
        assert!(is_trusted(&user, &project));
        assert!(is_trusted(&user, &other));
        assert!(!is_trusted(&user, &dir));
        let before = Hooks::load(&places);
        assert_eq!((before.defs().len(), before.skipped_untrusted), (0, 1));
        assert!(before.problems.iter().any(|problem| problem.contains("run /hooks trust again")));
        assert!(trust_project(&user, &project).unwrap());
        assert!(is_trusted(&user, &other));
        assert!(is_trusted(&user, &project));
        assert_eq!(Hooks::load(&places).defs().len(), 1);
        assert_eq!(std::fs::read_to_string(trusted_file(&user)).unwrap().lines().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_new_trust_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("private-trust");
        let (project, user) = (dir.join("proj"), dir.join("cfg"));
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(project.join(".jev/hooks.json"), "{}").unwrap();
        trust_project(&user, &project).unwrap();
        assert_eq!(std::fs::metadata(trusted_file(&user)).unwrap().permissions().mode() & 0o777, 0o600);
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
