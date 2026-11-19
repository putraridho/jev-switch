//! Multi-provider backend layer. A turn runs one CLI (`claude`, `codex`, ...) as a
//! child process that streams JSON lines on stdout. Everything the CLIs share, the
//! spawn, stdin hand-off, line reading, idle watchdog, stderr capture and error
//! mapping, lives once in [`run_turn`]; a [`Backend`] only says how to build its
//! command, how to read one output line, and how to classify its error text.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod grok;

use std::collections::VecDeque;
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
    /// The model's reasoning before it acts (extended thinking), never part of the answer.
    Thinking(String),
    /// What a tool call returned, as the CLI reported it (the text may be long; the screen
    /// shows a line of it). `ok` is false when the tool failed.
    ToolResult { ok: bool, text: String },
    /// The change an edit tool is about to make: the file, the lines it takes out and the
    /// lines it puts in (a whole new file has nothing taken out).
    Edit { path: String, removed: Vec<String>, added: Vec<String> },
    /// Assistant text: part of the answer.
    Text(String),
    /// A diagnostic about the turn that is never part of the answer: a tool call that was
    /// denied or failed, with what to do about it. Shown on stderr as it happens.
    Hint(String),
    Finished(Finish),
    Failed { message: String },
    Plan(PlanUsage),
    /// The CLI is waiting for the host to say whether it may run a tool. Only backends
    /// that are [`Backend::interactive`] produce it; [`run_turn_with`] answers it through
    /// the [`Approver`] and it never reaches `on_event`.
    PermissionRequest(PermissionRequest),
    /// The CLI no longer needs the answer to the request `id` (the turn was interrupted, a
    /// hook settled it, it timed out). [`run_turn_with`] drops the pending question, tells the
    /// [`Approver`] ([`Approver::withdrawn`]) and writes no answer for it; it never reaches
    /// `on_event`. An id that is not waiting (answered already, or not a question) is ignored.
    PermissionWithdrawn { id: String },
    /// A message the person typed while the turn ran (see [`Backend::steer_line`]) was picked
    /// up by the model: it is part of this turn now. The text is the message as sent.
    /// Backends report every user message they echo; [`run_turn_steered`] passes on only the
    /// ones it sent, and the person's own prompt is never one of them.
    Steered(String),
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
/// without any idle timeout: a person can take as long as they need. The turn keeps reading
/// the CLI meanwhile, so the decision may be abandoned: the future is dropped and
/// [`Approver::withdrawn`] follows. Only a future that yields can be abandoned, so an
/// implementation that waits for a person must wait asynchronously.
pub trait Approver: Send + Sync {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>>;

    /// The question for `req` was asked and then dropped without an answer: the CLI withdrew
    /// it, finished, or died. Take the question off the screen. Called after the `decide`
    /// future is dropped, once per such question, never for one that was answered.
    fn withdrawn(&self, _req: &PermissionRequest) {}
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
    /// Images to send with the prompt (absolute paths); see [`crate::attach`].
    pub images: Vec<PathBuf>,
    pub session_id: Option<String>,
    /// What the turn may do without asking; used by Claude Code, DeepSeek and Antigravity.
    pub mode: crate::mode::Mode,
}

/// The prompt with the attached images listed by path, for CLIs that take no image input
/// but open files themselves (Gemini, Antigravity, Grok).
pub(crate) fn prompt_with_images(req: &TurnRequest) -> String {
    if req.images.is_empty() {
        return req.prompt.clone();
    }
    let paths: Vec<String> = req.images.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    format!("{}\n\nAttached images (open them from these paths): {}", req.prompt, paths.join(", "))
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

    /// The line (without the newline) that adds `text` as a message from the person to the turn
    /// that is running, or `None` when this CLI cannot take one mid-turn. Only a backend that is
    /// [`Backend::interactive`] can: its stdin stays open. The CLI answers with
    /// [`Event::Steered`] once the model has the message, which is at the next step of the
    /// turn, not at once.
    fn steer_line(&self, _text: &str) -> Option<String> {
        None
    }

    /// Whether [`Backend::steer_line`] works for this backend as configured.
    fn steerable(&self) -> bool {
        false
    }

    /// Protocol messages produced while parsing stdout (for example ACP setup replies).
    /// Each includes its framing, and is sent before reading the next stdout line.
    fn take_responses(&self) -> Vec<String> {
        Vec::new()
    }

    /// Whether [`Backend::attach_mcp`] works for this CLI. Orchestration only picks a
    /// planner among providers that say yes: a planner that cannot call the delegate
    /// tool could not delegate anything. Gemini and Grok say no in this version.
    fn supports_mcp(&self) -> bool {
        false
    }

    /// Add `mcp` to this backend's commands. Only called when [`Backend::supports_mcp`].
    fn attach_mcp(&mut self, _mcp: &McpAttachment) {}

    /// Set the reasoning effort of this backend's turns. A CLI with no such control
    /// ignores it (see [`crate::effort::supported`]).
    fn set_effort(&mut self, _effort: crate::effort::Effort) {}

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

/// Create and verify the fallback directory without following a symlink.
fn create_private_prompt_dir(dir: &Path) -> std::io::Result<()> {
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    match builder.create(dir) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(dir)?;
    let private = metadata.file_type().is_dir();
    #[cfg(unix)]
    let private = {
        use std::os::unix::fs::MetadataExt;
        private && metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
    };
    if !private {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "prompt directory is not private"));
    }
    Ok(())
}

fn prompt_arg_in_fallback(dir: &Path, prompt: &str) -> PromptArg {
    if prompt.len() <= MAX_PROMPT_ARG_BYTES || create_private_prompt_dir(dir).is_err() {
        return PromptArg::Inline(prompt.to_string());
    }
    prompt_arg_in(dir, prompt)
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

/// [`prompt_arg_in`] in the state directory, or a verified private temp directory.
pub fn prompt_arg(prompt: &str) -> PromptArg {
    match crate::log::state_dir() {
        Some(dir) => prompt_arg_in(&dir.join("prompts"), prompt),
        None => prompt_arg_in_fallback(&std::env::temp_dir().join("jev-prompts"), prompt),
    }
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
/// How long the CLI may stay silent after its answer before the runner stops reading and
/// reaps it (see [`Pump::next_line`]).
const AFTER_ANSWER_WAIT: Duration = Duration::from_secs(2);

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

/// Reads the CLI's stdout to its end and acts on what it finds.
///
/// Outside a question the read has the idle watchdog: `idle` of silence is a stall, unless
/// the answer already came. During a question (a [`PermissionRequest`] being decided) stdout
/// is read too, with no watchdog, so a [`Event::PermissionWithdrawn`], the result, or the CLI
/// dying reach the runner while the person is still deciding. Events that arrive then are
/// held and delivered, in order, once the question is over.
struct Pump<'a> {
    backend: &'a dyn Backend,
    reader: BufReader<ChildStdout>,
    /// The line being read. Kept until it is complete: a read that lost a `select!` race
    /// leaves its partial bytes here and the next read continues them.
    raw: Vec<u8>,
    idle: Duration,
    on_event: &'a mut (dyn FnMut(&Event) + Send),
    plan: &'a mut Option<PlanUsage>,
    approver: &'a dyn Approver,
    feed: &'a mut StdinFeed,
    finished: Option<Finish>,
    /// Questions the CLI asked while another was being decided, oldest first.
    queue: VecDeque<PermissionRequest>,
    /// Messages from the person while the turn runs, to be added to it ([`Backend::steer_line`]).
    steer: Option<Steering<'a>>,
    /// Messages written to the CLI that it has not reported picking up yet, oldest first: what
    /// the person typed, and what was sent of it.
    sent: VecDeque<(String, String)>,
}

/// Messages the person types while a turn runs, and how one is made fit to send.
pub struct Steering<'a> {
    pub rx: mpsc::UnboundedReceiver<String>,
    /// The text as it may be sent (secrets blanked), or `None` to keep it out of this turn.
    pub prepare: &'a (dyn Fn(&str) -> Option<String> + Send + Sync),
}

/// The next message the person typed, waiting forever when there is no sender or it is gone.
async fn next_steer(steer: Option<&mut mpsc::UnboundedReceiver<String>>) -> String {
    match steer {
        Some(rx) => match rx.recv().await {
            Some(text) => text,
            // The sender is gone: nothing more will come.
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

/// What a line did to the wait it arrived in.
enum Signal {
    Failed(String),
    Finished,
    Withdrawn(String),
}

/// How the read of one line went.
enum Line {
    Events(Vec<Event>),
    Eof,
    Idle,
}

/// What a question did to the stream.
enum Asked {
    /// Over, answered or withdrawn: read on.
    Over,
    /// The stream ended or failed while it was waiting.
    Ended(Ended),
}

impl Pump<'_> {
    /// The line the last read completed, as events; `Eof` at the end of the stream.
    fn take_line(&mut self, read: std::io::Result<usize>) -> Line {
        if !matches!(read, Ok(n) if n > 0) && self.raw.is_empty() {
            return Line::Eof;
        }
        let events = self.backend.parse_line(&String::from_utf8_lossy(&self.raw));
        for response in self.backend.take_responses() {
            self.feed.send(response);
        }
        self.raw.clear();
        Line::Events(events)
    }

    /// The next line, under the idle watchdog. Once the answer is in, the CLI only has to
    /// exit: a short wait, not the full watchdog, so something it left running (a background
    /// shell, a monitor) cannot hold the session for minutes after the answer.
    async fn next_line(&mut self) -> Line {
        let wait = if self.finished.is_some() { self.idle.min(AFTER_ANSWER_WAIT) } else { self.idle };
        // One deadline for the line: a message typed meanwhile is not a sign of life from the CLI.
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let Self { reader, raw, steer, .. } = &mut *self;
            let steer = steer.as_mut().map(|s| &mut s.rx);
            // Writing a message to the CLI does not wait for the next line, so a message typed
            // while a long tool runs goes in at once.
            let typed = tokio::select! {
                read = tokio::time::timeout_at(deadline, reader.read_until(b'\n', raw)) => match read {
                    Ok(read) => return self.take_line(read),
                    Err(_) => return Line::Idle,
                },
                typed = next_steer(steer) => typed,
            };
            self.steer_in(typed);
        }
    }

    /// Add a message the person typed to the running turn. Not once the answer is in: stdin is
    /// closing then, and the message stays with the person's queue to run as the next turn.
    fn steer_in(&mut self, text: String) {
        if self.finished.is_some() {
            return;
        }
        let Some(prepared) = self.steer.as_ref().and_then(|s| (s.prepare)(&text)) else { return };
        if let Some(line) = self.backend.steer_line(&prepared) {
            self.feed.send(format!("{line}\n"));
            self.sent.push_back((text, prepared));
        }
    }

    fn deliver(&mut self, event: Event) {
        if let Event::Plan(p) = &event {
            *self.plan = Some(p.clone());
        }
        (self.on_event)(&event);
    }

    /// Apply one event. Something to show goes to `held` while a question is pending and
    /// to `on_event` at once otherwise; what changes the wait comes back as a [`Signal`].
    fn absorb(&mut self, event: Event, held: Option<&mut Vec<Event>>) -> Option<Signal> {
        match event {
            Event::Finished(f) => {
                self.finished = Some(f);
                self.queue.clear();
                // An interactive CLI keeps reading stdin until told there is no more.
                self.feed.close();
                Some(Signal::Finished)
            }
            Event::Failed { message } => Some(Signal::Failed(message)),
            Event::PermissionRequest(req) => {
                if self.finished.is_none() {
                    self.queue.push_back(req);
                }
                None
            }
            Event::PermissionWithdrawn { id } => {
                self.queue.retain(|q| q.id != id);
                Some(Signal::Withdrawn(id))
            }
            // The CLI echoes the person's own prompt too: only what was sent in is reported.
            Event::Steered(text) => {
                // Oldest first, but a message the CLI never echoes must not hide the later ones.
                if let Some(at) = self.sent.iter().position(|(_, sent)| sent.trim() == text.trim()) {
                    if let Some((typed, _)) = self.sent.remove(at) {
                        match held {
                            Some(held) => held.push(Event::Steered(typed)),
                            None => self.deliver(Event::Steered(typed)),
                        }
                    }
                }
                None
            }
            shown => {
                match held {
                    Some(held) => held.push(shown),
                    None => self.deliver(shown),
                }
                None
            }
        }
    }

    /// Decide `req` while reading on. Its answer is written once, when the decision
    /// completes. A withdrawal of `req`, the result, a failure or the end of the stream
    /// drops the decision instead and tells the approver; nothing is written then.
    async fn ask(&mut self, req: PermissionRequest) -> Asked {
        let approver = self.approver;
        let mut decision = approver.decide(&req);
        let mut held: Vec<Event> = Vec::new();
        let mut ended: Option<Ended> = None;
        let answer = loop {
            let Self { reader, raw, steer, .. } = &mut *self;
            let steer = steer.as_mut().map(|s| &mut s.rx);
            // A message typed while a question waits goes to the CLI as well: it has it at its next step.
            let line = tokio::select! {
                d = &mut decision => break Some(d),
                read = reader.read_until(b'\n', raw) => self.take_line(read),
                typed = next_steer(steer) => {
                    self.steer_in(typed);
                    continue;
                }
            };
            let Line::Events(events) = line else {
                ended = Some(Ended::Eof);
                break None;
            };
            let mut over = false;
            for event in events {
                match self.absorb(event, Some(&mut held)) {
                    Some(Signal::Failed(m)) => {
                        ended.get_or_insert(Ended::Failed(m));
                        over = true;
                    }
                    Some(Signal::Finished) => over = true,
                    Some(Signal::Withdrawn(id)) => over |= id == req.id,
                    None => {}
                }
            }
            if over {
                break None;
            }
        };
        match answer {
            Some(d) => {
                if let Some(line) = self.backend.permission_response(&req, &d) {
                    self.feed.send(format!("{line}\n"));
                }
            }
            None => {
                drop(decision);
                approver.withdrawn(&req);
            }
        }
        for event in held {
            self.deliver(event);
        }
        ended.map_or(Asked::Over, Asked::Ended)
    }

    /// The answer is in while a message the CLI has not taken sits in its pipe: stop reading now,
    /// so the CLI is not given the time to start on it (see `run_child`).
    fn answer_left_a_message(&self) -> bool {
        self.finished.is_some() && !self.sent.is_empty()
    }

    /// The result of the stream so far, once it is over.
    fn conclude(&mut self) -> Ended {
        self.finished.take().map_or(Ended::Eof, Ended::Finished)
    }

    async fn run(&mut self) -> Result<Ended, Idle> {
        'read: loop {
            let events = match self.next_line().await {
                Line::Events(events) => events,
                Line::Eof => break,
                Line::Idle if self.finished.is_some() => break,
                Line::Idle => return Err(Idle),
            };
            for event in events {
                if let Some(Signal::Failed(message)) = self.absorb(event, None) {
                    return Ok(Ended::Failed(message));
                }
            }
            if self.answer_left_a_message() {
                break;
            }
            while let Some(req) = self.queue.pop_front() {
                match self.ask(req).await {
                    // The answer may have come while the question was open.
                    Asked::Over if self.answer_left_a_message() => break 'read,
                    Asked::Over => {}
                    Asked::Ended(Ended::Failed(m)) => return Ok(Ended::Failed(m)),
                    Asked::Ended(_) => return Ok(self.conclude()),
                }
            }
        }
        Ok(self.conclude())
    }
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
    run_turn_steered(backend, req, idle_timeout, on_event, approver, None).await
}

/// [`run_turn_with`] that also takes messages the person types while the turn runs
/// (`steer`), for a backend that [`Backend::steerable`]. A message is written to the CLI as
/// it arrives and [`Event::Steered`] reports when the model has it. One that was not picked up
/// by the time the answer came is simply not reported: whoever sent it still has it.
pub async fn run_turn_steered(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
    steer: Option<Steering<'_>>,
) -> Result<TurnOutcome, TurnError> {
    // A guard, not a call after the await: a cancelled turn drops this future, and the
    // backend's files must go then too.
    let _cleanup = CleanupOnDrop(backend);
    run_child(backend, req, idle_timeout, on_event, approver, steer).await
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
#[cfg(any(test, unix))]
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
    steer: Option<Steering<'_>>,
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
    let mut pump = Pump {
        backend,
        reader: BufReader::new(stdout),
        raw: Vec::new(),
        idle: idle_timeout,
        on_event,
        plan: &mut plan,
        approver,
        feed: &mut feed,
        finished: None,
        queue: VecDeque::new(),
        steer: steer.filter(|_| backend.steerable()),
        sent: VecDeque::new(),
    };
    let ended = pump.run().await;
    // Messages written to the CLI that it never took before the answer: they are still the
    // person's to run, so the CLI must not start on them now (it would read them from the pipe
    // and the next turn would run them a second time).
    let not_taken = !pump.sent.is_empty();
    drop(pump);
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
        Ok(Ended::Finished(finish)) if not_taken => {
            // Dropping `group` kills everything the CLI started as well.
            let _ = child.start_kill();
            let _ = reap(&mut child).await;
            judge(finish, plan)
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

    #[cfg(unix)]
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
        TurnRequest { dir: std::env::temp_dir(), model: "m".into(), prompt: "hello".into(), images: vec![], session_id: None, mode: Default::default() }
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

    #[cfg(unix)]
    async fn read_pid(path: &Path) -> i32 {
        for _ in 0..200 {
            if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()) {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{path:?} never written");
    }

    #[cfg(unix)]
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
    #[cfg(unix)]
    fn shell_with_grandchild(dir: &Path) -> (String, PathBuf, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        let (shell, child) = (dir.join("shell.pid"), dir.join("child.pid"));
        let script = format!("sleep 60 & echo $! > {c}; echo $$ > {s}; wait", c = child.display(), s = shell.display());
        (script, shell, child)
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[tokio::test]
    async fn a_cli_that_lingers_after_its_answer_does_not_hold_the_turn_for_the_watchdog() {
        // The answer, then a CLI that keeps stdout open and stays silent (a background monitor).
        let request = req();
        let started = std::time::Instant::now();
        let r = run_turn(&Owned("echo FIN:ok; sleep 30".into()), &request, Duration::from_secs(60), &mut |_| {}).await;
        assert_eq!(r.unwrap().text, "ok");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn fallback_prompt_directory_must_be_private_and_not_a_symlink() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        let dir = scratch_dir("fallback");
        let prompt = "x".repeat(MAX_PROMPT_ARG_BYTES + 1);
        let arg = prompt_arg_in_fallback(&dir, &prompt);
        assert!(arg.file().is_some());
        let metadata = std::fs::symlink_metadata(&dir).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt.clone()));
        std::fs::remove_dir_all(&dir).unwrap();

        let target = scratch_dir("fallback-target");
        create_private_prompt_dir(&target).unwrap();
        symlink(&target, &dir).unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt.clone()));
        std::fs::remove_file(&dir).unwrap();
        std::fs::remove_dir(&target).unwrap();

        std::fs::write(&dir, "not a directory").unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt));
        std::fs::remove_file(&dir).unwrap();
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

    /// An interactive "CLI" that takes messages on stdin: the prompt is its first line, each
    /// later line `MSG:<text>` is echoed back as `ECHO:<text>` after a pause (the model taking
    /// it at its next step), and `GO` on a line ends the turn.
    struct Steerable(&'static str);

    impl Backend for Steerable {
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
            match line.split_once(':') {
                Some(("ECHO", t)) => vec![Event::Steered(t.into())],
                Some(("ASK", t)) => vec![Event::PermissionRequest(PermissionRequest { id: "q1".into(), tool: "Bash".into(), summary: t.into(), input: serde_json::json!({}), blocked_path: None })],
                Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn interactive(&self) -> bool {
            true
        }
        fn steerable(&self) -> bool {
            true
        }
        fn steer_line(&self, text: &str) -> Option<String> {
            Some(format!("MSG:{text}"))
        }
        fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
            Some(format!("{}\n", req.prompt))
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }

    async fn steered_turn(script: &'static str, send: &[&str], prepare: &(dyn Fn(&str) -> Option<String> + Send + Sync)) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
        let (tx, rx) = mpsc::unbounded_channel();
        for text in send {
            tx.send((*text).to_string()).unwrap();
        }
        let mut taken = Vec::new();
        let result = {
            let mut on_event = |e: &Event| {
                if let Event::Steered(t) = e {
                    taken.push(t.clone());
                }
            };
            run_turn_steered(&Steerable(script), &req(), Duration::from_secs(10), &mut on_event, &NoApprover, Some(Steering { rx, prepare })).await
        };
        drop(tx);
        (result, taken)
    }

    /// Reads the prompt, then each message: `MSG:x` is echoed as `ECHO:x`, `MSG:GO` ends the turn.
    const ECHO_ALL: &str = "read prompt; while read line; do case \"$line\" in MSG:GO) echo FIN:done; exit 0;; MSG:*) echo \"ECHO:${line#MSG:}\";; esac; done";

    #[tokio::test]
    async fn a_message_typed_during_the_turn_is_sent_in_and_reported_when_taken() {
        // `GO` ends the turn, so the first message is echoed before the answer comes.
        let (result, taken) = steered_turn(ECHO_ALL, &["also run the tests", "GO"], &|t| Some(t.to_string())).await;
        assert_eq!(result.unwrap().text, "done");
        assert_eq!(taken, vec!["also run the tests".to_string()], "only the message that was sent in is reported, as typed");
    }

    #[tokio::test]
    async fn a_message_the_policy_refuses_is_not_sent_and_not_reported() {
        let prepare = |t: &str| (!t.contains("secret")).then(|| t.to_string());
        let (result, taken) = steered_turn(ECHO_ALL, &["use the secret file", "GO"], &prepare).await;
        assert_eq!(result.unwrap().text, "done");
        assert!(taken.is_empty(), "{taken:?}");
    }

    #[tokio::test]
    async fn the_text_sent_may_differ_from_the_text_typed_and_the_typed_one_is_reported() {
        let prepare = |t: &str| Some(t.replace("sk-123", "[hidden]"));
        let (result, taken) = steered_turn(ECHO_ALL, &["use key sk-123", "GO"], &prepare).await;
        assert!(result.is_ok());
        assert_eq!(taken, vec!["use key sk-123".to_string()], "the person's own words come back, so their queue can match them");
    }

    #[tokio::test]
    async fn a_message_not_taken_before_the_answer_is_never_reported() {
        // The CLI answers without reading what was sent: it is still the person's to run next.
        let (result, taken) = steered_turn("read prompt; echo FIN:done", &["too late"], &|t| Some(t.to_string())).await;
        assert_eq!(result.unwrap().text, "done");
        assert!(taken.is_empty(), "{taken:?}");
    }

    #[tokio::test]
    async fn the_prompts_own_echo_is_not_a_steered_message() {
        // A CLI that echoes every user message, the first prompt included.
        let script = "read prompt; echo \"ECHO:$prompt\"; echo FIN:done";
        let (result, taken) = steered_turn(script, &[], &|t| Some(t.to_string())).await;
        assert!(result.is_ok());
        assert!(taken.is_empty(), "{taken:?}");
    }

    #[tokio::test]
    async fn a_message_the_cli_never_echoes_does_not_hide_the_ones_after_it() {
        // Takes `b` into the turn but never reports `a`.
        let script = "read prompt; while read line; do case \"$line\" in MSG:GO) echo FIN:done; exit 0;; MSG:b) echo ECHO:b;; esac; done";
        let (result, taken) = steered_turn(script, &["a", "b", "GO"], &|t| Some(t.to_string())).await;
        assert!(result.is_ok());
        assert_eq!(taken, vec!["b".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_cli_is_stopped_at_once_when_a_message_was_not_taken_so_it_runs_only_once_later() {
        let mark = std::env::temp_dir().join(format!("jev-steer-mark-{}", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        // Answers, then (after the time a model needs to start) would read the message left in the pipe and act on it.
        let script: &'static str = Box::leak(format!("read prompt; sleep 0.3; echo FIN:done; sleep 1.5; read line; touch {}", mark.display()).into_boxed_str());
        let (result, taken) = steered_turn(script, &["run the tests"], &|t| Some(t.to_string())).await;
        assert_eq!(result.unwrap().text, "done");
        assert!(taken.is_empty());
        tokio::time::sleep(Duration::from_millis(2200)).await;
        let acted = mark.exists();
        let _ = std::fs::remove_file(&mark);
        assert!(!acted, "the CLI acted on a message that stays in the person's queue");
    }

    /// An approver that never answers: the question stays open until the CLI ends it.
    #[cfg(unix)]
    struct NeverAnswers;

    #[cfg(unix)]
    impl Approver for NeverAnswers {
        fn decide<'a>(&'a self, _: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            Box::pin(std::future::pending())
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_answer_that_comes_while_a_question_is_open_also_stops_the_cli_at_once() {
        let mark = std::env::temp_dir().join(format!("jev-steer-ask-{}", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        let script: &'static str = Box::leak(format!("read prompt; echo ASK:ls; sleep 0.4; echo FIN:done; sleep 1.5; read line; touch {}", mark.display()).into_boxed_str());
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send("also run clippy".to_string()).unwrap();
        let mut on_event = |_: &Event| {};
        let prepare = |t: &str| Some(t.to_string());
        let out = run_turn_steered(&Steerable(script), &req(), Duration::from_secs(10), &mut on_event, &NeverAnswers, Some(Steering { rx, prepare: &prepare })).await;
        assert_eq!(out.unwrap().text, "done");
        tokio::time::sleep(Duration::from_millis(2200)).await;
        let acted = mark.exists();
        let _ = std::fs::remove_file(&mark);
        assert!(!acted, "the CLI got time to act on a message that stays in the person's queue");
    }

    #[tokio::test]
    async fn typing_does_not_keep_a_silent_cli_alive() {
        let (tx, rx) = mpsc::unbounded_channel();
        let feeder = tokio::spawn(async move {
            for i in 0..40 {
                if tx.send(format!("message {i}")).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        let mut on_event = |_: &Event| {};
        let prepare = |t: &str| Some(t.to_string());
        let started = std::time::Instant::now();
        let out = run_turn_steered(&Steerable("read prompt; sleep 30"), &req(), Duration::from_millis(700), &mut on_event, &NoApprover, Some(Steering { rx, prepare: &prepare })).await;
        feeder.abort();
        assert!(matches!(out, Err(ref e) if e.kind == ErrorKind::Stalled), "{out:?}");
        assert!(started.elapsed() < Duration::from_millis(3500), "typing kept it waiting: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_backend_that_cannot_take_messages_ignores_the_channel() {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send("hello".to_string()).unwrap();
        let mut on_event = |_: &Event| {};
        let prepare = |t: &str| Some(t.to_string());
        let out = run_turn_steered(&Fake("echo FIN:ok"), &req(), Duration::from_secs(5), &mut on_event, &NoApprover, Some(Steering { rx, prepare: &prepare })).await;
        assert_eq!(out.unwrap().text, "ok");
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

    /// An interactive "CLI": a shell script that asks questions and reads the answers from
    /// stdin. Output grammar: `ASK:<tool>:<summary>` (id `q1`), `ASKID:<id>:<tool>:<summary>`,
    /// `CANCEL:<id>`, `TEXT:<t>`, `TOOL:<t>`, `FIN:<text>`. The answers it receives are
    /// `<id>:ALLOW` (also for always) and `<id>:DENY:<reason>`.
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
            let ask = |id: &str, rest: &str| {
                let (tool, summary) = rest.split_once(':').unwrap_or((rest, ""));
                vec![Event::PermissionRequest(PermissionRequest {
                    id: id.into(),
                    tool: tool.into(),
                    summary: summary.into(),
                    input: serde_json::json!({ "command": summary }),
                    blocked_path: None,
                })]
            };
            match line.split_once(':') {
                Some(("ASK", rest)) => ask("q1", rest),
                Some(("ASKID", rest)) => {
                    let (id, rest) = rest.split_once(':').unwrap_or((rest, ""));
                    ask(id, rest)
                }
                Some(("CANCEL", id)) => vec![Event::PermissionWithdrawn { id: id.into() }],
                Some(("TEXT", t)) => vec![Event::Text(t.into())],
                Some(("TOOL", t)) => vec![Event::Tool(t.into())],
                Some(("FIN", text)) => vec![Event::Finished(Finish { text: text.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
            Some(match decision {
                Decision::Allow | Decision::AllowAlways => format!("{}:ALLOW", req.id),
                Decision::Deny { reason } => format!("{}:DENY:{reason}", req.id),
            })
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }

    const ASK_AND_REPORT: &str =
        r#"read prompt; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in q1:ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#q1:DENY:}";; esac"#;

    /// An approver that answers after `wait`, and remembers what it was asked, what it was
    /// told was withdrawn, and (in `log`) what happened in which order.
    struct Scripted {
        wait: Duration,
        decision: Decision,
        asked: Mutex<Vec<(String, String)>>,
        withdrawn: Mutex<Vec<String>>,
        log: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(wait_ms: u64, decision: Decision) -> Self {
            Self {
                wait: Duration::from_millis(wait_ms),
                decision,
                asked: Mutex::new(Vec::new()),
                withdrawn: Mutex::new(Vec::new()),
                log: Mutex::new(Vec::new()),
            }
        }

        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }

        fn withdrawn_ids(&self) -> Vec<String> {
            self.withdrawn.lock().unwrap().clone()
        }
    }

    impl Approver for Scripted {
        fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            Box::pin(async move {
                self.asked.lock().unwrap().push((req.tool.clone(), req.summary.clone()));
                self.note(format!("asked {}", req.id));
                tokio::time::sleep(self.wait).await;
                self.note(format!("decided {}", req.id));
                self.decision.clone()
            })
        }

        fn withdrawn(&self, req: &PermissionRequest) {
            self.withdrawn.lock().unwrap().push(req.id.clone());
            self.note(format!("withdrawn {}", req.id));
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

    #[cfg(unix)]
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

    // ----------------------------------------------- reading on while a question waits

    /// Run `script` with every event it produces logged next to the approver's own log.
    async fn run_logged(script: &str, approver: &Scripted, idle_ms: u64) -> Result<TurnOutcome, TurnError> {
        let mut on_event = |e: &Event| approver.note(format!("event {e:?}"));
        run_turn_with(&Asking(script.into()), &req(), Duration::from_millis(idle_ms), &mut on_event, approver).await
    }

    /// The script ends by copying whatever the runner wrote to its stdin into `file`, so a test
    /// can see exactly which answers reached the CLI. The runner closes stdin after the result.
    fn recording(dir: &Path, body: &str) -> (String, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join("received");
        (format!("{body}; cat > {}", file.display()), file)
    }

    fn received(file: &Path) -> String {
        std::fs::read_to_string(file).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_withdrawn_question_is_dropped_unanswered_and_the_ui_is_told() {
        let dir = scratch_dir("withdraw");
        // The person never answers (a minute); the CLI withdraws the question, with a progress
        // line before it, and goes on.
        let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "TOOL:Read a"; sleep 0.2; echo "CANCEL:q1"; echo "FIN:done""#);
        let approver = Scripted::new(60_000, Decision::Allow);
        let started = std::time::Instant::now();
        let r = run_logged(&script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "done");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert_eq!(approver.withdrawn_ids(), ["q1"]);
        assert_eq!(received(&file), "", "no answer is written for a withdrawn question");
        // The event that arrived while the question was pending still reaches the UI, after it.
        assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "withdrawn q1", r#"event Tool("Read a")"#]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn events_arriving_while_a_question_waits_are_delivered_in_order_after_it() {
        let approver = Scripted::new(400, Decision::Allow);
        let script = r#"read p; echo "ASK:Bash:x"; echo "TEXT:one"; echo "TOOL:two"; echo "TEXT:three"; read a; echo "TOOL:four"; echo "FIN:$a""#;
        let r = run_logged(script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "q1:ALLOW");
        assert_eq!(
            *approver.log.lock().unwrap(),
            ["asked q1", "decided q1", r#"event Text("one")"#, r#"event Tool("two")"#, r#"event Text("three")"#, r#"event Tool("four")"#]
        );
    }

    #[tokio::test]
    async fn the_cli_ending_while_a_question_waits_drops_the_question() {
        let approver = Scripted::new(60_000, Decision::Allow);
        let started = std::time::Instant::now();
        let r = run_logged(r#"read p; echo "ASK:Bash:x"; echo "TEXT:last words"; exit 3"#, &approver, 5000).await;
        let err = r.unwrap_err();
        assert!(err.message.contains("exited"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        assert_eq!(approver.withdrawn_ids(), ["q1"]);
        assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "withdrawn q1", r#"event Text("last words")"#]);
    }

    #[tokio::test]
    async fn a_result_while_a_question_waits_drops_the_question_and_keeps_the_answer() {
        let dir = scratch_dir("finish-pending");
        let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "FIN:already done""#);
        let approver = Scripted::new(60_000, Decision::Allow);
        let r = run_logged(&script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "already done");
        assert_eq!(approver.withdrawn_ids(), ["q1"]);
        assert_eq!(received(&file), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_idle_watchdog_runs_again_once_a_question_was_withdrawn() {
        let approver = Scripted::new(60_000, Decision::Allow);
        let r = run_logged(r#"read p; echo "ASK:Bash:x"; sleep 0.1; echo "CANCEL:q1"; exec sleep 30"#, &approver, 400).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
        assert_eq!(approver.withdrawn_ids(), ["q1"]);
    }

    #[tokio::test]
    async fn two_questions_one_after_the_other_are_each_answered_once_with_their_own_id() {
        let dir = scratch_dir("sequential");
        let (script, file) = recording(
            &dir,
            r#"read p; echo "ASKID:a:Bash:one"; read r1; echo "ASKID:b:Bash:two"; read r2; echo "FIN:$r1|$r2""#,
        );
        let approver = Scripted::new(30, Decision::Allow);
        let r = run_logged(&script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "a:ALLOW|b:ALLOW");
        assert_eq!(received(&file), "");
        assert!(approver.withdrawn_ids().is_empty());
        assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a", "asked b", "decided b"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn questions_asked_together_are_decided_one_at_a_time_in_order() {
        let approver = Scripted::new(100, Decision::Deny { reason: "no".into() });
        let script = r#"read p; echo "ASKID:a:Bash:one"; echo "ASKID:b:Bash:two"; read r1; read r2; echo "FIN:$r1|$r2""#;
        let r = run_logged(script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "a:DENY:no|b:DENY:no");
        assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a", "asked b", "decided b"]);
    }

    #[tokio::test]
    async fn a_waiting_question_that_is_withdrawn_is_never_asked() {
        let approver = Scripted::new(300, Decision::Allow);
        let script = r#"read p; echo "ASKID:a:Bash:one"; echo "ASKID:b:Bash:two"; echo "CANCEL:b"; read r1; echo "FIN:$r1""#;
        let r = run_logged(script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "a:ALLOW");
        assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a"], "b was withdrawn before its turn");
        assert!(approver.withdrawn_ids().is_empty(), "it was never shown, so there is nothing to take down");
    }

    #[tokio::test]
    async fn a_cancel_for_another_id_changes_nothing() {
        let dir = scratch_dir("other-id");
        let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "CANCEL:zzz"; read a; echo "FIN:$a""#);
        let approver = Scripted::new(200, Decision::Allow);
        let r = run_logged(&script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "q1:ALLOW", "the question stayed and was answered");
        assert!(approver.withdrawn_ids().is_empty());
        assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "decided q1"]);
        assert_eq!(received(&file), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cancel_after_the_answer_is_ignored() {
        let approver = Scripted::new(0, Decision::Allow);
        let script = r#"read p; echo "ASK:Bash:x"; read a; echo "CANCEL:q1"; echo "FIN:$a""#;
        let r = run_logged(script, &approver, 5000).await;
        assert_eq!(r.unwrap().text, "q1:ALLOW");
        assert!(approver.withdrawn_ids().is_empty());
    }
}
