//! Multi-provider backend layer. A turn runs one CLI (`claude`, `codex`, ...) as a
//! child process that streams JSON lines on stdout. Everything the CLIs share, the
//! spawn, stdin hand-off, line reading, idle watchdog, stderr capture and error
//! mapping, lives once in [`run_turn`]; a [`Backend`] only says how to build its
//! command, how to read one output line, and how to classify its error text.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod grok;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::anthropic::{is_limit_message, is_limit_phrase};

/// Which service answers a turn. Several ids can share one CLI (DeepSeek is
/// `claude` pointed at another API), so this is not the same as the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    Claude,
    DeepSeek,
    Codex,
    Gemini,
    Grok,
}

impl ProviderId {
    /// Stable lowercase name used in logs, env vars and config.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::DeepSeek => "deepseek",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "deepseek" => Ok(Self::DeepSeek),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            "grok" => Ok(Self::Grok),
            other => Err(format!("unknown provider `{other}` (claude, deepseek, codex, gemini, grok)")),
        }
    }
}

/// Token counts for one turn. `input_tokens` is everything sent (fresh plus cached,
/// so providers compare on one scale) and `cached_input_tokens` is the cached share.
/// `context_tokens` is the size of the last request, when the CLI reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub context_tokens: Option<u64>,
}

/// Why a turn failed, as far as the caller needs to react differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Usage limit or rate limit; `resets_at` is the CLI's own wording, if any.
    Quota { resets_at: Option<String> },
    /// The model or feature is not part of the user's plan.
    NotEntitled,
    /// Not logged in, or the login or key expired.
    Auth,
    /// The CLI finished with an empty answer.
    Empty,
    /// No output for the whole idle timeout.
    Stalled,
    /// The CLI refuses to run in this directory (Codex outside a git repo, Gemini CLI
    /// in an untrusted workspace). Another provider would work, so a turn fails over.
    UnusableHere,
    Other,
}

/// What a plan-limit window looked like when the CLI last reported it. Informational.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanUsage {
    pub window: String,
    pub utilization: f64,
    pub resets_at: Option<u64>,
}

/// The terminal success event of a CLI's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finish {
    pub text: String,
    pub session_id: Option<String>,
    pub usage: Usage,
}

/// One thing a backend found in one stdout line.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// One-line summary of a tool call, like "Bash cargo test".
    Tool(String),
    /// Assistant text (or a hint) worth showing while the turn runs.
    Text(String),
    Finished(Finish),
    Failed { message: String },
    Plan(PlanUsage),
    /// The CLI is waiting for the host to say whether it may run a tool. Only backends
    /// that are [`Backend::interactive`] produce it; [`run_turn_with`] answers it through
    /// the [`Approver`] and it never reaches `on_event`.
    PermissionRequest(PermissionRequest),
}

/// A tool call the CLI wants approval for.
#[derive(Clone, PartialEq)]
pub struct PermissionRequest {
    /// The CLI's id for the question; the answer must carry it back.
    pub id: String,
    pub tool: String,
    /// One line for the person: the command, or the path (and size) for a file write.
    pub summary: String,
    /// The tool's input exactly as the CLI sent it. It can hold secrets: it is only ever
    /// echoed back to the CLI, never logged or shown.
    pub input: serde_json::Value,
    /// The path outside the allowed folders that made the CLI ask, when it said.
    pub blocked_path: Option<String>,
}

impl std::fmt::Debug for PermissionRequest {
    /// Leaves out the summary and the input: both can hold a secret, and `{:?}` ends up in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionRequest").field("id", &self.id).field("tool", &self.tool).finish_non_exhaustive()
    }
}

/// What happens to a [`PermissionRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it this once.
    Allow,
    /// Run it, and the approver has remembered a rule so the CLI does not ask again.
    AllowAlways,
    /// Do not run it; the model is told `reason`.
    Deny { reason: String },
}

/// Why a request is denied when nobody can be asked.
pub const NO_ONE_TO_ASK: &str = "no one to ask: this session has no terminal to approve tool calls";

/// Whoever answers [`PermissionRequest`]s while a turn runs. The turn waits for the answer
/// without any idle timeout: a person can take as long as they need.
pub trait Approver: Send + Sync {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>>;
}

/// The approver of a run that has no one to ask: every request is denied at once.
pub struct NoApprover;

impl Approver for NoApprover {
    fn decide<'a>(&'a self, _: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async { Decision::Deny { reason: NO_ONE_TO_ASK.to_string() } })
    }
}

/// Everything a backend needs to run one turn.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub dir: PathBuf,
    pub model: String,
    pub prompt: String,
    pub session_id: Option<String>,
}

/// A successful turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    pub text: String,
    pub session_id: Option<String>,
    pub usage: Usage,
    pub plan: Option<PlanUsage>,
}

/// A failed turn: `kind` for callers that react, `message` for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnError {
    pub kind: ErrorKind,
    pub message: String,
}

impl TurnError {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TurnError {}

/// An MCP server to attach to one turn (orchestration's delegate tool). One value
/// carries both spellings the CLIs need: Claude reads a JSON config FILE
/// (`--mcp-config`), Codex takes the server as `-c mcp_servers.<name>.*` overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAttachment {
    /// The server's name; tools reach the model as `mcp__<name>__<tool>`.
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    /// The JSON config file Claude Code is pointed at; the caller writes it before the turn.
    pub config_path: PathBuf,
}

/// What one CLI contributes. Object-safe and synchronous on purpose: the only
/// async part (process IO) is the same for every CLI and lives in [`run_turn`].
pub trait Backend: Send + Sync {
    fn provider(&self) -> ProviderId;

    /// Fully configured command (program, args, env, `current_dir = req.dir`).
    /// Stdin, stdout and stderr are NOT set; the runner pipes all three.
    fn command(&self, req: &TurnRequest) -> Command;

    /// Text written to the child's stdin, which is then closed. `None` when the
    /// prompt travels in the args instead.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        Some(req.prompt.clone())
    }

    /// Parse ONE stdout line. Non-JSON or irrelevant lines give an empty Vec.
    fn parse_line(&self, line: &str) -> Vec<Event>;

    /// Whether this CLI asks the host before running a tool ([`Event::PermissionRequest`])
    /// and waits for [`Backend::permission_response`] on stdin. Such a CLI keeps stdin open
    /// until it has answered the turn; every other CLI gets its prompt and EOF at once.
    fn interactive(&self) -> bool {
        false
    }

    /// The line (without the newline) that answers `req` on the CLI's stdin. Only called
    /// when [`Backend::interactive`].
    fn permission_response(&self, _req: &PermissionRequest, _decision: &Decision) -> Option<String> {
        None
    }

    /// Whether [`Backend::attach_mcp`] works for this CLI. Orchestration only picks a
    /// planner among providers that say yes: a planner that cannot call the delegate
    /// tool could not delegate anything. Gemini and Grok say no in this version.
    fn supports_mcp(&self) -> bool {
        false
    }

    /// Add `mcp` to this backend's commands. Only called when [`Backend::supports_mcp`].
    fn attach_mcp(&mut self, _mcp: &McpAttachment) {}

    /// Classify an error message from the CLI (a `Failed` text or a stderr tail).
    /// Implementations call [`classify_common`] first and may add their own rules.
    fn classify_error(&self, message: &str) -> ErrorKind;

    /// Called by [`run_turn`] once the turn is over, whatever its result: remove
    /// anything [`Backend::command`] left on disk (see [`PromptFiles`]).
    fn cleanup(&self) {}
}

/// True when `needle` (a number) appears in `hay` not glued to other digits or
/// letters, so "429" matches "HTTP 429" but not "a4291b".
pub(crate) fn has_token(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The "resets ..." tail of a limit message ("resets 3pm", "resets Nov 24", "resets 5pm
/// (Asia/Jakarta)"), or the epoch in the older "... limit reached|<epoch>" form. Kept
/// as the CLI's own wording.
fn resets_tail(message: &str) -> Option<String> {
    // ASCII lowercasing keeps byte offsets identical to the original.
    let lower = message.to_ascii_lowercase();
    if let Some(i) = lower.find("resets") {
        let tail = message[i + "resets".len()..].lines().next().unwrap_or("");
        let tail = tail.trim().trim_start_matches([':', '-']).trim();
        return (!tail.is_empty()).then(|| tail.chars().take(80).collect());
    }
    let (_, epoch) = lower.split_once("limit reached|")?;
    let digits: String = epoch.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// True when `lower` contains any of `phrases`.
pub(crate) fn has_any(lower: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| lower.contains(p))
}

/// Matches only PHRASES a CLI uses for these failures, never a bare word: error
/// text routinely mentions "quota", "upgrade" or "expired" in other senses, and a
/// wrong match would mark a provider unusable or stop failover.
fn is_quota_text(lower: &str) -> bool {
    is_limit_phrase(lower)
        || has_any(lower, &["resource_exhausted", "too many requests"])
        || (lower.contains("rate limit")
            && (has_any(lower, &["exceed", "reached", "too many"]) || has_token(lower, "hit")))
        || (lower.contains("quota") && has_any(lower, &["exceed", "exhaust", "reached", "insufficient"]))
        || has_token(lower, "429")
}

fn is_entitlement_text(lower: &str) -> bool {
    let about_the_plan = has_any(lower, &["plan", "subscription", "tier", "account"]);
    has_any(lower, &["not supported when using", "not available on your plan"])
        || (about_the_plan && has_any(lower, &["upgrade your", "upgrade to a", "not included in your"]))
}

fn is_auth_text(lower: &str) -> bool {
    has_token(lower, "401")
        || has_token(lower, "unauthorized")
        || has_any(lower, &["not logged in", "please log in", "invalid api key", "login expired", "session expired"])
        || has_any(lower, &["authentication failed", "authentication required", "authentication error"])
        || (lower.contains("token") && lower.contains("expired"))
}

/// "I will not run here": a workspace-trust or git-repository check. Codex says
/// "Not inside a trusted directory and --skip-git-repo-check was not specified" and
/// Gemini CLI "is not running in a trusted directory ... --skip-trust".
fn is_unusable_here_text(lower: &str) -> bool {
    ["trusted directory", "trusted folder", "trusted workspace", "untrusted", "skip-git-repo-check", "skip-trust"]
        .iter()
        .any(|p| lower.contains(p))
}

/// Phrases every CLI uses for the errors callers act on. Backends call this first
/// so quota, plan, login and "cannot run here" failures read the same whichever
/// CLI said them.
pub fn classify_common(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase().replace('\u{2019}', "'");
    if is_quota_text(&lower) {
        Some(ErrorKind::Quota { resets_at: resets_tail(message) })
    } else if is_entitlement_text(&lower) {
        Some(ErrorKind::NotEntitled)
    } else if is_auth_text(&lower) {
        Some(ErrorKind::Auth)
    } else if is_unusable_here_text(&lower) {
        Some(ErrorKind::UnusableHere)
    } else {
        None
    }
}

// ------------------------------------------------------- prompts as arguments

/// Longest prompt handed to a CLI as ONE command-line argument. Linux refuses a
/// single argument over 131072 bytes (`E2BIG`, "Argument list too long"), so stay well under.
pub const MAX_PROMPT_ARG_BYTES: usize = 100_000;

/// A prompt as a CLI that has no stdin hand-off has to receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptArg {
    /// Short enough to be the argument itself.
    Inline(String),
    /// Too long: the prompt is in `path` (private to the user) and `instruction` is a
    /// short argument that tells the model to read it.
    File { path: PathBuf, instruction: String },
}

impl PromptArg {
    /// The text for the CLI's prompt argument: the prompt, or the instruction to read the file.
    pub fn text(&self) -> &str {
        match self {
            Self::Inline(prompt) => prompt,
            Self::File { instruction, .. } => instruction,
        }
    }

    /// The file holding a long prompt, if there is one.
    pub fn file(&self) -> Option<&Path> {
        match self {
            Self::Inline(_) => None,
            Self::File { path, .. } => Some(path),
        }
    }
}

/// Where long prompts are written: `<state dir>/prompts`, else the OS temp dir.
fn default_prompt_dir() -> PathBuf {
    crate::log::state_dir().map_or_else(|| std::env::temp_dir().join("jev-prompts"), |d| d.join("prompts"))
}

/// A new, uniquely named file in `dir` (created if missing), readable by the owner only.
fn write_private(dir: &Path, content: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let name = format!("prompt-{}-{nanos}-{}.txt", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let path = dir.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&path)?, content.as_bytes())?;
    Ok(path)
}

/// The one rule for a prompt that must travel as a command-line argument (Grok,
/// Antigravity): the prompt itself up to [`MAX_PROMPT_ARG_BYTES`], else a private
/// file in `dir` plus a short instruction. The caller must remove the file, see
/// [`PromptFiles`]. If the file cannot be written the prompt is passed inline anyway,
/// so the CLI's own error is what the user sees rather than a silent loss.
pub fn prompt_arg_in(dir: &Path, prompt: &str) -> PromptArg {
    if prompt.len() <= MAX_PROMPT_ARG_BYTES {
        return PromptArg::Inline(prompt.to_string());
    }
    match write_private(dir, prompt) {
        Ok(path) => {
            let instruction = format!(
                "Your task is written in the file {}. Read that file completely and carry out what it says.",
                path.display()
            );
            PromptArg::File { path, instruction }
        }
        Err(_) => PromptArg::Inline(prompt.to_string()),
    }
}

/// [`prompt_arg_in`] in the default directory.
pub fn prompt_arg(prompt: &str) -> PromptArg {
    prompt_arg_in(&default_prompt_dir(), prompt)
}

/// Owns the prompt files one backend instance wrote, so they are removed when the
/// turn ends ([`Backend::cleanup`]) and, as a safety net, when the backend is dropped
/// (a turn cancelled mid-way). The files can hold code and must never be left behind.
#[derive(Default)]
pub struct PromptFiles {
    dir: Option<PathBuf>,
    written: Mutex<Vec<PathBuf>>,
}

impl PromptFiles {
    /// Write into `dir` instead of the default directory.
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir), written: Mutex::default() }
    }

    /// [`prompt_arg_in`], remembering the file for [`PromptFiles::cleanup`].
    pub fn arg(&self, prompt: &str) -> PromptArg {
        let arg = match &self.dir {
            Some(dir) => prompt_arg_in(dir, prompt),
            None => prompt_arg(prompt),
        };
        if let Some(path) = arg.file() {
            self.written.lock().unwrap_or_else(PoisonError::into_inner).push(path.to_path_buf());
        }
        arg
    }

    /// Remove every file written so far.
    pub fn cleanup(&self) {
        for path in self.written.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for PromptFiles {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// How much of stderr is kept in memory, and how much of it goes in a message.
const STDERR_KEEP: usize = 8192;
const STDERR_SHOWN: usize = 400;
/// After the CLI has answered or died, how long to wait for it to be reaped.
const REAP_WAIT: Duration = Duration::from_secs(2);

/// How the stdout stream ended.
enum Ended {
    Finished(Finish),
    Failed(String),
    Eof,
}

/// Marker for "no stdout line within the idle timeout".
struct Idle;

/// Everything written to the CLI's stdin goes through one task, so a big prompt cannot
/// deadlock against a CLI that starts printing before it reads all of stdin, and so the
/// runner can keep writing (permission answers) while it reads stdout. Stdin closes when
/// the feed is closed or dropped; some CLIs wait forever for end of input.
struct StdinFeed(Option<mpsc::UnboundedSender<String>>);

impl StdinFeed {
    /// Send `payload`, then close stdin at once unless `keep_open`.
    fn start(stdin: Option<ChildStdin>, payload: Option<String>, keep_open: bool) -> Self {
        let Some(mut stdin) = stdin else { return Self(None) };
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                // A CLI that exits early closes the pipe; its exit is reported by the runner.
                if stdin.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let mut feed = Self(Some(tx));
        if let Some(payload) = payload {
            feed.send(payload);
        }
        if !keep_open {
            feed.close();
        }
        feed
    }

    fn send(&self, text: String) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(text);
        }
    }

    /// Close stdin once what was sent has been written.
    fn close(&mut self) {
        self.0 = None;
    }
}

/// Drain stderr concurrently, keeping only the tail: an unread pipe fills up and
/// blocks the child, and the tail is all an error message needs.
fn collect_stderr(stderr: Option<ChildStderr>) -> JoinHandle<String> {
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

fn last_chars(s: &str, n: usize) -> String {
    let s = s.trim();
    let skip = s.chars().count().saturating_sub(n);
    s.chars().skip(skip).collect()
}

/// Read stdout until the CLI fails, ends, or goes quiet for `idle`. After a
/// `Finished` the stream is still read to EOF so the process can exit cleanly,
/// but a CLI that then hangs does not lose the answer it already gave.
///
/// While a [`PermissionRequest`] waits for its answer nothing is being read, so no idle
/// timeout runs: the watchdog only covers the wait for the CLI's next line.
async fn pump(
    backend: &dyn Backend,
    stdout: ChildStdout,
    idle: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    plan: &mut Option<PlanUsage>,
    approver: &dyn Approver,
    feed: &mut StdinFeed,
) -> Result<Ended, Idle> {
    let mut reader = BufReader::new(stdout);
    let mut finished: Option<Finish> = None;
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match tokio::time::timeout(idle, reader.read_until(b'\n', &mut raw)).await {
            Err(_) if finished.is_some() => break,
            Err(_) => return Err(Idle),
            Ok(Ok(n)) if n > 0 => {}
            Ok(_) => break,
        }
        for event in backend.parse_line(&String::from_utf8_lossy(&raw)) {
            match event {
                Event::Finished(f) => {
                    finished = Some(f);
                    // An interactive CLI keeps reading stdin until told there is no more.
                    feed.close();
                }
                Event::Failed { message } => return Ok(Ended::Failed(message)),
                Event::PermissionRequest(req) => {
                    let decision = approver.decide(&req).await;
                    if let Some(line) = backend.permission_response(&req, &decision) {
                        feed.send(format!("{line}\n"));
                    }
                }
                Event::Plan(p) => {
                    on_event(&Event::Plan(p.clone()));
                    *plan = Some(p);
                }
                shown => on_event(&shown),
            }
        }
    }
    Ok(finished.map_or(Ended::Eof, Ended::Finished))
}

/// An answer can still be a failure: empty, or the Claude CLI's way of saying the
/// plan is used up (normal answer text, no error flag).
fn judge(finish: Finish, plan: Option<PlanUsage>) -> Result<TurnOutcome, TurnError> {
    if finish.text.trim().is_empty() {
        return Err(TurnError::new(ErrorKind::Empty, "the CLI returned an empty answer"));
    }
    if is_limit_message(&finish.text) {
        let resets_at = resets_tail(&finish.text);
        return Err(TurnError::new(
            ErrorKind::Quota { resets_at },
            format!("usage limit reached: {}", finish.text.trim()),
        ));
    }
    Ok(TurnOutcome { text: finish.text, session_id: finish.session_id, usage: finish.usage, plan })
}

/// Wait for the child to be reaped, killing it if it will not exit.
async fn reap(child: &mut Child) -> Option<std::process::ExitStatus> {
    match tokio::time::timeout(REAP_WAIT, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            let _ = child.start_kill();
            None
        }
    }
}

/// A stream that ended with no result: the stderr tail is the best explanation.
async fn explain_eof(child: &mut Child, stderr: JoinHandle<String>) -> String {
    let status = reap(child).await;
    let tail = tokio::time::timeout(REAP_WAIT, stderr).await.ok().and_then(Result::ok).unwrap_or_default();
    let tail = last_chars(&tail, STDERR_SHOWN);
    if !tail.is_empty() {
        return tail;
    }
    match status {
        Some(s) => format!("the CLI exited ({s}) without a result"),
        None => "the CLI ended without a result".into(),
    }
}

/// Run one turn through `backend` and return its answer or a classified error.
/// `on_event` sees `Tool`, `Text` and `Plan` events while the turn runs. The child
/// is killed if the future is dropped, and after `idle_timeout` of silence.
pub async fn run_turn(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
) -> Result<TurnOutcome, TurnError> {
    run_turn_with(backend, req, idle_timeout, on_event, &NoApprover).await
}

/// [`run_turn`] with someone to answer the tool approvals of an [`Backend::interactive`]
/// CLI. For any other backend `approver` is never asked.
pub async fn run_turn_with(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
) -> Result<TurnOutcome, TurnError> {
    // A guard, not a call after the await: a cancelled turn drops this future, and the
    // backend's files must go then too.
    let _cleanup = CleanupOnDrop(backend);
    run_child(backend, req, idle_timeout, on_event, approver).await
}

/// Runs [`Backend::cleanup`] when the turn ends or its future is dropped.
struct CleanupOnDrop<'a>(&'a dyn Backend);

impl Drop for CleanupOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cleanup();
    }
}

/// The negative process-group id `kill(2)` takes to signal a whole group. `None` for
/// a pid that would signal something else: `kill(-0)` is `kill(0)`, our own group.
fn group_signal_target(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|p| *p > 0).map(|p| -p)
}

/// Owns the process group of a running CLI. The CLIs spawn helpers (shells, test
/// runs, MCP servers) and `kill_on_drop` reaches only the direct child, so when the
/// turn ends badly or is cancelled (the future dropped) the whole group gets SIGKILL.
/// A turn that finished cleanly [`disarm`](Self::disarm)s it: background work an
/// agent started on purpose is not ours to kill. Off unix there are no groups and
/// the direct-child kill remains.
struct GroupKill(Option<u32>);

impl GroupKill {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(target) = self.0.and_then(group_signal_target) {
            // SAFETY: plain `kill(2)` on a negative pid, which is a process group this
            // runner created with `process_group(0)`; no memory is involved.
            unsafe {
                libc::kill(target, libc::SIGKILL);
            }
        }
    }
}

async fn run_child(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
) -> Result<TurnOutcome, TurnError> {
    let mut cmd = backend.command(req);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // Its own process group, so the whole tree can be killed (see [`GroupKill`]).
    #[cfg(unix)]
    cmd.process_group(0);
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let other = |m: String| TurnError::new(ErrorKind::Other, m);
    let mut child = cmd.spawn().map_err(|e| other(format!("failed to run `{program}` (is it installed?): {e}")))?;
    let mut group = GroupKill(if cfg!(unix) { child.id() } else { None });
    let stdout = child.stdout.take().ok_or_else(|| other(format!("`{program}` gave no stdout")))?;
    let mut feed = StdinFeed::start(child.stdin.take(), backend.stdin_payload(req), backend.interactive());
    let stderr = collect_stderr(child.stderr.take());

    let mut plan = None;
    let ended = pump(backend, stdout, idle_timeout, on_event, &mut plan, approver, &mut feed).await;
    let classified = |message: String| TurnError::new(backend.classify_error(&message), message);
    match ended {
        Err(Idle) => {
            let _ = child.start_kill();
            let secs = idle_timeout.as_secs_f32();
            Err(TurnError::new(ErrorKind::Stalled, format!("`{program}` printed nothing for {secs}s; stopped it")))
        }
        Ok(Ended::Failed(message)) => {
            let _ = child.start_kill();
            Err(classified(message))
        }
        Ok(Ended::Finished(finish)) => {
            if reap(&mut child).await.is_some() {
                group.disarm();
            }
            judge(finish, plan)
        }
        Ok(Ended::Eof) => Err(classified(explain_eof(&mut child, stderr).await)),
    }
}

/// Alive means running: a killed process that nobody has reaped yet is a zombie, which
/// `kill(pid, 0)` still reports as there. Unix only, like the process groups it tests.
#[cfg(all(test, unix))]
pub(crate) fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only asks whether the process exists.
    let exists = unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    exists && !is_zombie(pid)
}

/// Linux: the state letter in `/proc/<pid>/stat` (after the `(name)`, which may hold spaces).
#[cfg(all(test, target_os = "linux"))]
fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'))
}

/// macOS and the BSDs have no `/proc`: ask `ps` for the state.
#[cfg(all(test, unix, not(target_os = "linux")))]
fn is_zombie(pid: i32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim_start().starts_with('Z'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use super::process_alive as alive;

    /// A backend whose "CLI" is a shell script printing canned lines, so the runner
    /// is tested without any real CLI. Line grammar: `FIN:text`, `FAIL:message`,
    /// `TOOL:summary`, `TEXT:text`.
    struct Fake(&'static str);

    impl Backend for Fake {
        fn provider(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn command(&self, req: &TurnRequest) -> Command {
            let mut c = Command::new("sh");
            c.arg("-c").arg(self.0).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            let line = line.trim_end();
            let ev = match line.split_once(':') {
                Some(("FIN", t)) => Event::Finished(Finish {
                    text: t.into(),
                    session_id: Some("s1".into()),
                    usage: Usage { output_tokens: 7, ..Usage::default() },
                }),
                Some(("FAIL", m)) => Event::Failed { message: m.into() },
                Some(("TOOL", t)) => Event::Tool(t.into()),
                Some(("TEXT", t)) => Event::Text(t.into()),
                _ => return vec![],
            };
            vec![ev]
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            classify_common(message).unwrap_or(ErrorKind::Other)
        }
    }

    fn req() -> TurnRequest {
        TurnRequest { dir: std::env::temp_dir(), model: "m".into(), prompt: "hello".into(), session_id: None }
    }

    async fn run(script: &'static str, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
        let mut seen = Vec::new();
        let mut on_event = |e: &Event| seen.push(e.clone());
        let r = run_turn(&Fake(script), &req(), Duration::from_millis(idle_ms), &mut on_event).await;
        (r, seen)
    }

    #[tokio::test]
    async fn finished_turn_streams_events_and_returns_outcome() {
        let (r, seen) = run("echo noise; echo TOOL:Bash ls; echo TEXT:hi; echo FIN:done", 5000).await;
        let out = r.unwrap();
        assert_eq!(out.text, "done");
        assert_eq!(out.session_id.as_deref(), Some("s1"));
        assert_eq!(out.usage.output_tokens, 7);
        assert_eq!(seen, vec![Event::Tool("Bash ls".into()), Event::Text("hi".into())]);
    }

    #[tokio::test]
    async fn prompt_is_delivered_on_stdin_and_stdin_is_closed() {
        // `cat` only returns once stdin is closed, so this also proves the close.
        let (r, _) = run("echo FIN:$(cat)", 5000).await;
        assert_eq!(r.unwrap().text, "hello");
    }

    #[tokio::test]
    async fn failed_event_is_classified() {
        let (r, _) = run("echo 'FAIL:You have hit your usage limit, resets 5pm'", 5000).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Quota { resets_at: Some("5pm".into()) });
    }

    #[tokio::test]
    async fn silent_child_is_killed_as_stalled() {
        let started = std::time::Instant::now();
        let (r, _) = run("echo TEXT:start; exec sleep 30", 300).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// A backend whose script is built at run time (it needs a scratch path).
    struct Owned(String);

    impl Backend for Owned {
        fn provider(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn command(&self, req: &TurnRequest) -> Command {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&self.0).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            match line.trim_end().split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }

    async fn read_pid(path: &Path) -> i32 {
        for _ in 0..200 {
            if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()) {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{path:?} never written");
    }

    async fn all_dead(pids: &[i32]) -> bool {
        for _ in 0..200 {
            if pids.iter().all(|p| !alive(*p)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    /// A shell that starts a sleeping grandchild and waits for it; both pids go to files.
    fn shell_with_grandchild(dir: &Path) -> (String, PathBuf, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        let (shell, child) = (dir.join("shell.pid"), dir.join("child.pid"));
        let script = format!("sleep 60 & echo $! > {c}; echo $$ > {s}; wait", c = child.display(), s = shell.display());
        (script, shell, child)
    }

    #[tokio::test]
    async fn dropping_the_turn_kills_the_cli_and_its_children() {
        let dir = scratch_dir("group-drop");
        let (script, shell_file, child_file) = shell_with_grandchild(&dir);
        let backend = Owned(script);
        let request = req();
        let mut on_event = |_: &Event| {};
        let turn = run_turn(&backend, &request, Duration::from_secs(60), &mut on_event);
        let pids = async { (read_pid(&shell_file).await, read_pid(&child_file).await) };
        let (shell, child) = tokio::select! {
            _ = turn => panic!("the turn cannot end by itself"),
            pids = pids => pids,
        };
        // The select dropped the turn: the shell and the sleep it started must both go.
        assert!(all_dead(&[shell, child]).await, "shell alive: {}, grandchild alive: {}", alive(shell), alive(child));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_stalled_cli_takes_its_children_with_it() {
        let dir = scratch_dir("group-stall");
        let (script, shell_file, child_file) = shell_with_grandchild(&dir);
        let request = req();
        let r = run_turn(&Owned(script), &request, Duration::from_millis(300), &mut |_| {}).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
        let pids = [read_pid(&shell_file).await, read_pid(&child_file).await];
        assert!(all_dead(&pids).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_failing_cli_takes_its_children_with_it() {
        let dir = scratch_dir("group-fail");
        std::fs::create_dir_all(&dir).unwrap();
        let child_file = dir.join("child.pid");
        let script = format!("sleep 60 & echo $! > {}; echo 'FAIL:boom'; wait", child_file.display());
        struct Fails(Owned);
        impl Backend for Fails {
            fn provider(&self) -> ProviderId {
                ProviderId::Claude
            }
            fn command(&self, req: &TurnRequest) -> Command {
                self.0.command(req)
            }
            fn parse_line(&self, line: &str) -> Vec<Event> {
                match line.trim_end().split_once(':') {
                    Some(("FAIL", m)) => vec![Event::Failed { message: m.into() }],
                    _ => vec![],
                }
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
        }
        let request = req();
        let r = run_turn(&Fails(Owned(script)), &request, Duration::from_secs(5), &mut |_| {}).await;
        assert!(r.is_err());
        assert!(all_dead(&[read_pid(&child_file).await]).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_clean_finish_leaves_background_work_the_agent_started_alone() {
        let dir = scratch_dir("group-clean");
        std::fs::create_dir_all(&dir).unwrap();
        let child_file = dir.join("child.pid");
        let script = format!("sleep 30 >/dev/null 2>&1 & echo $! > {}; echo FIN:ok", child_file.display());
        let request = req();
        let r = run_turn(&Owned(script), &request, Duration::from_secs(5), &mut |_| {}).await;
        assert_eq!(r.unwrap().text, "ok");
        let child = read_pid(&child_file).await;
        assert!(alive(child), "a finished turn does not kill its group");
        // SAFETY: tidying our own test process.
        unsafe { libc::kill(child, libc::SIGKILL) };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_alive_tells_running_from_zombie_from_gone() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        assert!(alive(pid), "a running process is alive");
        assert!(alive(std::process::id() as i32), "so is this one");
        child.kill().unwrap();
        // Killed but not yet waited for: a zombie, which is not alive (the test helper must
        // not call it running, or the group tests would pass vacuously on any OS).
        for _ in 0..200 {
            if !alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive(pid), "a zombie is not alive");
        child.wait().unwrap();
        assert!(!alive(pid), "a reaped process is gone");
    }

    #[test]
    fn the_group_signal_never_targets_our_own_group_or_everything() {
        assert_eq!(group_signal_target(4242), Some(-4242));
        assert_eq!(group_signal_target(0), None);
        assert_eq!(group_signal_target(u32::MAX), None);
    }

    #[tokio::test]
    async fn a_dropped_turn_still_cleans_up_the_backends_files() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Flag(Arc<AtomicBool>);
        impl Backend for Flag {
            fn provider(&self) -> ProviderId {
                ProviderId::Claude
            }
            fn command(&self, req: &TurnRequest) -> Command {
                let mut c = Command::new("sh");
                c.arg("-c").arg("sleep 30").current_dir(&req.dir);
                c
            }
            fn parse_line(&self, _: &str) -> Vec<Event> {
                vec![]
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
            fn cleanup(&self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cleaned = Arc::new(AtomicBool::new(false));
        let request = req();
        let backend = Flag(cleaned.clone());
        let gone = tokio::time::timeout(Duration::from_millis(100), run_turn(&backend, &request, Duration::from_secs(60), &mut |_| {})).await;
        assert!(gone.is_err(), "the turn was still running when it was dropped");
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn empty_answer_is_an_error() {
        let (r, _) = run("echo 'FIN:   '", 5000).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Empty);
    }

    #[tokio::test]
    async fn limit_text_as_answer_is_quota() {
        let (r, _) = run("echo \"FIN:You've hit your session limit · resets 5pm (Asia/Jakarta)\"", 5000).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Quota { resets_at: Some("5pm (Asia/Jakarta)".into()) });
        assert!(e.message.contains("usage limit reached"));
    }

    #[tokio::test]
    async fn exit_without_result_uses_stderr_tail() {
        let (r, _) = run("echo 'Error: not logged in' >&2; exit 1", 5000).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Auth);
        assert!(e.message.contains("not logged in"));
    }

    #[tokio::test]
    async fn exit_without_result_or_stderr_names_the_exit() {
        let (r, _) = run("exit 3", 5000).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Other);
        assert!(e.message.contains("without a result"));
    }

    #[tokio::test]
    async fn stderr_tail_is_trimmed_to_the_last_400_chars() {
        let (r, _) = run("head -c 5000 /dev/zero | tr '\\0' x >&2; echo END >&2", 5000).await;
        let m = r.unwrap_err().message;
        assert!(m.chars().count() <= 400 && m.ends_with("END"));
    }

    #[tokio::test]
    async fn missing_program_is_other() {
        struct Missing;
        impl Backend for Missing {
            fn provider(&self) -> ProviderId {
                ProviderId::Codex
            }
            fn command(&self, _: &TurnRequest) -> Command {
                Command::new("/nonexistent/jev-cli")
            }
            fn parse_line(&self, _: &str) -> Vec<Event> {
                vec![]
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
        }
        let r = run_turn(&Missing, &req(), Duration::from_secs(1), &mut |_| {}).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Other);
    }

    #[test]
    fn provider_ids_round_trip() {
        for p in [ProviderId::Claude, ProviderId::DeepSeek, ProviderId::Codex, ProviderId::Gemini, ProviderId::Grok] {
            assert_eq!(p.as_str().parse::<ProviderId>(), Ok(p));
        }
        assert_eq!("  Codex ".parse::<ProviderId>(), Ok(ProviderId::Codex));
        assert!("openai".parse::<ProviderId>().is_err());
    }

    #[test]
    fn common_quota_phrases() {
        let quota = |m: &str| matches!(classify_common(m), Some(ErrorKind::Quota { .. }));
        assert!(quota("You've hit your weekly limit"));
        assert!(quota("Usage limit reached"));
        assert!(quota("rate limit exceeded"));
        assert!(quota("Quota exceeded for this project"));
        assert!(quota("HTTP 429 Too Many Requests"));
        assert_eq!(
            classify_common("You've hit your session limit · resets 5pm (Asia/Jakarta)"),
            Some(ErrorKind::Quota { resets_at: Some("5pm (Asia/Jakarta)".into()) })
        );
        assert_eq!(
            classify_common("Claude AI usage limit reached|1794853451"),
            Some(ErrorKind::Quota { resets_at: Some("1794853451".into()) })
        );
        assert_eq!(classify_common("quota exceeded"), Some(ErrorKind::Quota { resets_at: None }));
    }

    #[test]
    fn real_claude_limit_notices_are_quota_with_their_reset_time() {
        let quota = |resets: Option<&str>| Some(ErrorKind::Quota { resets_at: resets.map(String::from) });
        for (text, resets) in [
            ("5-hour limit reached \u{2219} resets 3pm", Some("3pm")),
            ("Weekly limit reached \u{2219} resets Nov 24", Some("Nov 24")),
            ("Opus weekly limit reached", None),
            ("You've hit your session limit \u{b7} resets 5pm (Asia/Jakarta)", Some("5pm (Asia/Jakarta)")),
            ("You\u{2019}ve hit your session limit \u{b7} resets 5pm (Asia/Jakarta)", Some("5pm (Asia/Jakarta)")),
            ("Claude AI usage limit reached|1794853451", Some("1794853451")),
            ("5-hour limit reached|1794853451", Some("1794853451")),
            ("Approaching usage limit", None),
            ("limit reached", None),
            ("You have reached your weekly limit", None),
            ("You've reached your usage limit \u{b7} resets 9am", Some("9am")),
            ("monthly limit \u{2219} resets Dec 1", Some("Dec 1")),
        ] {
            assert_eq!(classify_common(text), quota(resets), "{text:?}");
        }
    }

    #[test]
    fn common_entitlement_and_auth_phrases() {
        assert_eq!(classify_common("model is not supported when using ChatGPT"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("Not available on your plan"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("Please upgrade your plan to continue"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("401 Unauthorized"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("Not logged in"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("token expired"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("Please log in again"), Some(ErrorKind::Auth));
    }

    #[test]
    fn cannot_run_here_wording_is_unusable_here() {
        let here = |m: &str| classify_common(m) == Some(ErrorKind::UnusableHere);
        // Codex outside a git repository.
        assert!(here("Not inside a trusted directory and --skip-git-repo-check was not specified."));
        // Gemini CLI in an untrusted workspace.
        assert!(here(
            "Gemini CLI is not running in a trusted directory. To proceed, either use `--skip-trust`, set the \
             `GEMINI_CLI_TRUST_WORKSPACE=true` environment variable, or trust this directory in interactive mode."
        ));
        assert!(here("this is an untrusted workspace"));
        // Quota and login wording still win over it.
        assert!(matches!(classify_common("usage limit, resets 5pm"), Some(ErrorKind::Quota { .. })));
        assert_eq!(classify_common("please log in"), Some(ErrorKind::Auth));
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-prompt-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_prompt_under_the_limit_is_passed_through_unchanged() {
        let dir = scratch_dir("short");
        let prompt = "x".repeat(MAX_PROMPT_ARG_BYTES);
        let arg = prompt_arg_in(&dir, &prompt);
        assert_eq!(arg, PromptArg::Inline(prompt.clone()));
        assert_eq!(arg.text(), prompt);
        assert!(arg.file().is_none());
        assert!(!dir.exists(), "no file, not even the directory, is made for a short prompt");
    }

    #[test]
    fn a_long_prompt_goes_to_a_private_file_and_the_arg_is_a_short_instruction() {
        let dir = scratch_dir("long");
        let prompt = format!("{}\u{e9}end", "y".repeat(MAX_PROMPT_ARG_BYTES));
        let arg = prompt_arg_in(&dir, &prompt);
        let path = arg.file().expect("long prompt is written to a file").to_path_buf();
        assert!(arg.text().len() < 500, "{}", arg.text());
        assert!(arg.text().contains(&path.display().to_string()));
        assert!(arg.text().contains("Read that file completely"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), prompt);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Two prompts never share a file.
        let other = prompt_arg_in(&dir, &prompt);
        assert_ne!(other.file(), Some(path.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_files_cleanup_removes_what_was_written_and_drop_is_a_safety_net() {
        let dir = scratch_dir("cleanup");
        let long = "z".repeat(MAX_PROMPT_ARG_BYTES + 1);
        let files = PromptFiles::in_dir(dir.clone());
        let a = files.arg(&long).file().unwrap().to_path_buf();
        assert!(a.exists());
        files.cleanup();
        assert!(!a.exists());
        let b = files.arg(&long).file().unwrap().to_path_buf();
        drop(files);
        assert!(!b.exists(), "dropping the owner removes its files too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_turn_cleans_up_the_backends_files_after_every_kind_of_turn() {
        struct Writes {
            files: PromptFiles,
            path: Mutex<Option<PathBuf>>,
            script: &'static str,
        }
        impl Backend for Writes {
            fn provider(&self) -> ProviderId {
                ProviderId::Grok
            }
            fn command(&self, req: &TurnRequest) -> Command {
                *self.path.lock().unwrap() = self.files.arg(&req.prompt).file().map(Path::to_path_buf);
                let mut c = Command::new("sh");
                c.arg("-c").arg(self.script).current_dir(&req.dir);
                c
            }
            fn parse_line(&self, line: &str) -> Vec<Event> {
                match line.trim_end().split_once(':') {
                    Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                    _ => vec![],
                }
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
            fn cleanup(&self) {
                self.files.cleanup();
            }
        }
        let dir = scratch_dir("runner");
        let mut request = req();
        request.prompt = "p".repeat(MAX_PROMPT_ARG_BYTES + 1);
        for script in ["echo FIN:ok", "exit 1"] {
            let b = Writes { files: PromptFiles::in_dir(dir.clone()), path: Mutex::new(None), script };
            let r = run_turn(&b, &request, Duration::from_secs(5), &mut |_| {}).await;
            assert_eq!(r.is_ok(), script.contains("FIN"));
            let path = b.path.lock().unwrap().clone().expect("the long prompt was written");
            assert!(!path.exists(), "{script}: left {path:?} behind");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn common_ignores_unrelated_text() {
        assert_eq!(classify_common("segmentation fault"), None);
        assert_eq!(classify_common("read 14291 bytes"), None);
        assert_eq!(classify_common(""), None);
    }

    #[test]
    fn common_matches_phrases_and_never_bare_words() {
        use ErrorKind::{Auth, NotEntitled};
        let kind = |m: &str| classify_common(m).map(|k| match k {
            ErrorKind::Quota { .. } => "quota",
            NotEntitled => "entitled",
            Auth => "auth",
            ErrorKind::UnusableHere => "here",
            _ => "other",
        });
        let should_match = [
            ("quota", "You have hit your usage limit"),
            ("quota", "You've hit your session limit"),
            ("quota", "rate limit exceeded"),
            ("quota", "Rate limit reached for requests"),
            ("quota", "we hit the rate limit"),
            ("quota", "quota exceeded"),
            ("quota", "you have exhausted your quota"),
            ("quota", "insufficient quota"),
            ("quota", "RESOURCE_EXHAUSTED: try later"),
            ("quota", "Too Many Requests"),
            ("quota", "status 429"),
            ("entitled", "model is not supported when using ChatGPT"),
            ("entitled", "Not available on your plan"),
            ("entitled", "Upgrade your subscription to use this model"),
            ("entitled", "upgrade to a paid tier"),
            ("entitled", "Grok Build is not included in your X Premium plan"),
            ("auth", "HTTP 401"),
            ("auth", "Unauthorized"),
            ("auth", "Not logged in"),
            ("auth", "Please log in again"),
            ("auth", "Invalid API key"),
            ("auth", "authentication failed or timed out"),
            ("auth", "Authentication required"),
            ("auth", "the OAuth token has expired"),
            ("auth", "login expired"),
            ("auth", "Your session expired"),
        ];
        for (want, text) in should_match {
            assert_eq!(kind(text), Some(want), "{text:?}");
        }
        let must_not_match = [
            "the upgrade guide is in docs/upgrading.md",
            "upgrade the dependency to 2.0",
            "expired cache entries were removed",
            "quota calculation function",
            "authenticate the user in the handler",
            "authentication module not found",
            "the rate limit middleware is missing",
            "rate limit handling for the client",
            "when the timer resets the counter goes to zero",
            "resets the state of the form",
            "the resets handler",
            "limit the query to 10 rows",
            "reached your destination in 10 minutes",
            "file not included in the build",
            "error 4010 in module",
            "request id 14290",
        ];
        for text in must_not_match {
            assert_eq!(kind(text), None, "{text:?}");
        }
    }

    // ------------------------------------------------------------ host approvals

    /// An interactive "CLI": a shell script that asks one question and reads the answer from
    /// stdin. Output grammar: `ASK:<tool>:<summary>`, `FIN:<text>`. The answers it receives
    /// are `ALLOW` (also for always) and `DENY:<reason>`.
    struct Asking(String);

    impl Backend for Asking {
        fn provider(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn command(&self, req: &TurnRequest) -> Command {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&self.0).current_dir(&req.dir);
            c
        }
        fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
            Some(format!("{}\n", req.prompt))
        }
        fn interactive(&self) -> bool {
            true
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            let line = line.trim_end();
            match line.split_once(':') {
                Some(("ASK", rest)) => {
                    let (tool, summary) = rest.split_once(':').unwrap_or((rest, ""));
                    vec![Event::PermissionRequest(PermissionRequest {
                        id: "q1".into(),
                        tool: tool.into(),
                        summary: summary.into(),
                        input: serde_json::json!({ "command": summary }),
                        blocked_path: None,
                    })]
                }
                Some(("FIN", text)) => vec![Event::Finished(Finish { text: text.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn permission_response(&self, _: &PermissionRequest, decision: &Decision) -> Option<String> {
            Some(match decision {
                Decision::Allow | Decision::AllowAlways => "ALLOW".to_string(),
                Decision::Deny { reason } => format!("DENY:{reason}"),
            })
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }

    const ASK_AND_REPORT: &str =
        r#"read prompt; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#DENY:}";; esac"#;

    /// An approver that answers after `wait`, and remembers what it was asked.
    struct Scripted {
        wait: Duration,
        decision: Decision,
        asked: Mutex<Vec<(String, String)>>,
    }

    impl Scripted {
        fn new(wait_ms: u64, decision: Decision) -> Self {
            Self { wait: Duration::from_millis(wait_ms), decision, asked: Mutex::new(Vec::new()) }
        }
    }

    impl Approver for Scripted {
        fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            Box::pin(async move {
                self.asked.lock().unwrap().push((req.tool.clone(), req.summary.clone()));
                tokio::time::sleep(self.wait).await;
                self.decision.clone()
            })
        }
    }

    async fn run_asking(script: &str, approver: &dyn Approver, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
        let mut seen = Vec::new();
        let mut on_event = |e: &Event| seen.push(e.clone());
        let r = run_turn_with(&Asking(script.into()), &req(), Duration::from_millis(idle_ms), &mut on_event, approver).await;
        (r, seen)
    }

    #[tokio::test]
    async fn an_allowed_request_lets_the_cli_go_on_and_is_not_shown_as_an_event() {
        let approver = Scripted::new(0, Decision::Allow);
        let (r, seen) = run_asking(ASK_AND_REPORT, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "ran");
        assert_eq!(*approver.asked.lock().unwrap(), vec![("Bash".to_string(), "cargo test --quiet".to_string())]);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[tokio::test]
    async fn always_reaches_the_cli_as_an_allow() {
        let (r, _) = run_asking(ASK_AND_REPORT, &Scripted::new(0, Decision::AllowAlways), 5000).await;
        assert_eq!(r.unwrap().text, "ran");
    }

    #[tokio::test]
    async fn a_denial_carries_its_reason_to_the_cli() {
        let approver = Scripted::new(0, Decision::Deny { reason: "not today".into() });
        let (r, _) = run_asking(ASK_AND_REPORT, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "refused not today");
    }

    #[tokio::test]
    async fn the_idle_watchdog_does_not_run_while_the_person_decides() {
        // The question takes four times the idle timeout to answer; the turn must not be cut off.
        let approver = Scripted::new(800, Decision::Allow);
        let (r, _) = run_asking(ASK_AND_REPORT, &approver, 200).await;
        assert_eq!(r.unwrap().text, "ran");
        // And it still guards the CLI's own silence afterwards.
        let (r, _) = run_asking(r#"read p; echo "ASK:Bash:x"; read a; exec sleep 30"#, &Scripted::new(0, Decision::Allow), 300).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
    }

    #[tokio::test]
    async fn without_an_approver_every_request_is_denied_and_nothing_hangs() {
        let mut on_event = |_: &Event| {};
        let started = std::time::Instant::now();
        let r = run_turn(&Asking(ASK_AND_REPORT.into()), &req(), Duration::from_secs(5), &mut on_event).await;
        assert_eq!(r.unwrap().text, format!("refused {NO_ONE_TO_ASK}"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn stdin_stays_open_for_answers_and_closes_once_the_turn_has_finished() {
        let dir = scratch_dir("interactive-close");
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("exited");
        // After its answer the "CLI" reads until stdin closes, then leaves a marker.
        let script = format!(
            r#"read prompt; echo "ASK:Bash:x"; read ans; echo "FIN:ok"; cat > /dev/null; touch {}"#,
            marker.display()
        );
        let started = std::time::Instant::now();
        let (r, _) = run_asking(&script, &Scripted::new(0, Decision::Allow), 5000).await;
        assert_eq!(r.unwrap().text, "ok");
        assert!(started.elapsed() < Duration::from_millis(1500), "stdin was not closed after the result: {:?}", started.elapsed());
        assert!(marker.exists(), "the CLI should have seen end of input and exited by itself");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_turn_dropped_while_a_question_waits_kills_the_cli() {
        let dir = scratch_dir("interactive-drop");
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("cli.pid");
        let script = format!(r#"echo $$ > {}; read prompt; echo "ASK:Bash:x"; read ans; echo FIN:late"#, pid_file.display());
        let approver = Scripted::new(60_000, Decision::Allow);
        let request = req();
        let mut on_event = |_: &Event| {};
        let backend = Asking(script);
        let turn = run_turn_with(&backend, &request, Duration::from_millis(300), &mut on_event, &approver);
        let waited = async {
            let pid = read_pid(&pid_file).await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            pid
        };
        let pid = tokio::select! {
            _ = turn => panic!("the turn waits for the answer"),
            pid = waited => pid,
        };
        assert!(all_dead(&[pid]).await, "the CLI must not outlive the cancelled turn");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
