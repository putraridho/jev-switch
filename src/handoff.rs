//! Hand-off between providers. Every provider CLI keeps its own session, so when a
//! turn goes to a provider that missed earlier turns (they ran elsewhere) it has
//! to be caught up. The project folder is shared on disk, so the new model can
//! re-read real files; what it lacks is the dialogue and the intent. This module
//! rebuilds exactly that, deterministically and without any LLM call: a short
//! plain-text block (prompts verbatim, replies shrunk, files and commands listed)
//! that the caller prepends to the next prompt.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::backend::ProviderId;

/// One finished (or failed) turn, as the hand-off needs to remember it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub prompt: String,
    pub provider: ProviderId,
    pub model: String,
    pub reply: String,
    /// `Event::Tool` summaries seen during the turn, in order.
    pub tools: Vec<String>,
    pub ok: bool,
}

/// The conversation as the router saw it, plus what each provider's own CLI
/// session has lived through. `seen` is a prefix length into `log`: a provider has
/// seen `log[..seen]`. A prefix is enough because a provider that runs a turn is
/// (by the caller's contract) caught up first, so it never has holes.
#[derive(Debug, Clone, Default)]
pub struct Sessions {
    ids: HashMap<ProviderId, String>,
    seen: HashMap<ProviderId, usize>,
    log: Vec<TurnRecord>,
    /// The name the person gave the session (`/rename`), shown in place of its first message.
    name: Option<String>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The CLI session to resume for `p`, if it has one.
    pub fn session_id(&self, p: ProviderId) -> Option<&str> {
        self.ids.get(&p).map(String::as_str)
    }

    pub fn set_session_id(&mut self, p: ProviderId, id: String) {
        self.ids.insert(p, id);
    }

    /// Append a turn. The provider that ran it lived it, so it is marked as having
    /// seen everything up to and including this turn. This relies on the caller
    /// having caught the provider up (see [`Sessions::mark_seen`]) before running it.
    pub fn record(&mut self, rec: TurnRecord) {
        let p = rec.provider;
        self.log.push(rec);
        self.mark_seen(p);
    }

    /// Append a turn that got NO answer. Unlike [`Sessions::record`] this marks the
    /// provider caught up only if it already was before this turn: a provider that was
    /// still owed earlier turns when its attempt failed (login, stall, wrong folder)
    /// never read them, so it must get them next time. A provider that was current keeps
    /// its place (its own CLI session has seen the prompt).
    pub fn record_failed(&mut self, rec: TurnRecord) {
        let p = rec.provider;
        let was_current = self.missed_turns(p) == 0;
        self.log.push(rec);
        if was_current {
            self.mark_seen(p);
        }
    }

    /// Append a turn the user cancelled. Like [`Sessions::record_undo`] it marks NOBODY as
    /// caught up, not even `provider`, which ran it: that provider's own CLI session was
    /// killed mid-turn and never recorded it, so its next prompt carries the note too.
    pub fn record_interrupted(&mut self, prompt: &str, provider: ProviderId, model: &str, tools: Vec<String>) {
        self.log.push(TurnRecord {
            prompt: prompt.to_string(),
            provider,
            model: model.to_string(),
            reply: INTERRUPTED_NOTE.to_string(),
            tools,
            ok: false,
        });
    }

    /// Declare `p` caught up with the whole log. Called after [`Sessions::catch_up`]
    /// has been put in front of a prompt.
    pub fn mark_seen(&mut self, p: ProviderId) {
        self.seen.insert(p, self.log.len());
    }

    /// How many logged turns `p`'s own session has not lived through.
    pub fn missed_turns(&self, p: ProviderId) -> usize {
        self.log.len().saturating_sub(self.seen.get(&p).copied().unwrap_or(0))
    }

    /// The catch-up block for `p`, or `None` when it missed nothing. Ends with
    /// `Now the user says:` so the caller can append the new prompt directly. It is
    /// shrunk step by step until it fits `budget_tokens` (see [`LEVELS`]); if even
    /// the smallest form is over budget it is returned anyway, because losing a
    /// user prompt is worse than a few extra tokens.
    pub fn catch_up(&self, p: ProviderId, git_status: &str, budget_tokens: usize) -> Option<String> {
        self.catch_up_as(p, None, git_status, budget_tokens)
    }

    /// Like [`Sessions::catch_up`], but the header also names the receiving model
    /// (`You are: codex/gpt-6-luna`), so it can tell its own turns from the listed ones.
    pub fn catch_up_for(&self, p: ProviderId, model: &str, git_status: &str, budget_tokens: usize) -> Option<String> {
        self.catch_up_as(p, Some(model), git_status, budget_tokens)
    }

    fn catch_up_as(&self, p: ProviderId, model: Option<&str>, git_status: &str, budget_tokens: usize) -> Option<String> {
        let missed = &self.log[self.log.len() - self.missed_turns(p)..];
        if missed.is_empty() {
            return None;
        }
        let mut text = String::new();
        for level in LEVELS {
            text = render(missed, &identity(p, model), git_status, level);
            if estimate_tokens(&text) <= budget_tokens {
                break;
            }
        }
        Some(text)
    }

    /// Rough token size of the whole dialogue, for context-window checks.
    pub fn context_tokens_estimate(&self) -> usize {
        let chars: usize = self
            .log
            .iter()
            .map(|r| char_count(&r.prompt) + char_count(&r.reply) + r.tools.iter().map(|t| char_count(t)).sum::<usize>())
            .sum();
        chars / CHARS_PER_TOKEN
    }

    /// Number of logged turns (answered, failed, interrupted, and undo notes).
    pub fn turns(&self) -> usize {
        self.log.len()
    }

    /// The name given with `/rename`, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Name the session (`None` takes the name away). Whitespace is collapsed.
    pub fn set_name(&mut self, name: Option<&str>) {
        self.name = name.map(|n| n.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|n| !n.is_empty());
    }

    /// What the session is called in a list: its name, else what the person first asked.
    pub fn title(&self) -> Option<String> {
        self.name.clone().or_else(|| self.first_prompt())
    }

    /// What the person first asked, one line: the name of the session in a list.
    pub fn first_prompt(&self) -> Option<String> {
        self.log.iter().map(|r| r.prompt.trim()).find(|p| !p.is_empty()).map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// Providers that ran a turn, in the order they first appeared.
    pub fn providers_used(&self) -> Vec<ProviderId> {
        let mut out: Vec<ProviderId> = Vec::new();
        for r in &self.log {
            if !out.contains(&r.provider) {
                out.push(r.provider);
            }
        }
        out
    }

    /// The prompt and reply of the last turn that was answered, for Jev's correction question.
    pub fn last_answered(&self) -> Option<(&str, &str)> {
        self.log.iter().rev().find(|r| r.ok).map(|r| (r.prompt.as_str(), r.reply.as_str()))
    }

    /// Note that the user undid the changes of the previous turn. Unlike
    /// [`Sessions::record_failed`] this marks nobody as caught up, not even the provider
    /// that ran the turn: its own CLI session still believes the files are as it left
    /// them, so its next prompt carries this note and tells it to look again.
    pub fn record_undo(&mut self) {
        let (provider, model) = self.log.last().map_or((ProviderId::Claude, String::new()), |r| (r.provider, r.model.clone()));
        self.log.push(TurnRecord {
            prompt: "/undo".into(),
            provider,
            model,
            reply: UNDO_NOTE.into(),
            tools: Vec::new(),
            ok: false,
        });
    }

    /// Forget everything (`/new`): sessions, what was seen, and the log.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The last `max_turns` turns as `user: ...\nassistant: ...` pairs, newest last,
    /// with replies cut to `reply_chars`. Short context for cheap classifiers that
    /// must tell what a short follow-up like "do it" refers to.
    pub fn recent_context(&self, max_turns: usize, reply_chars: usize) -> String {
        let start = self.log.len().saturating_sub(max_turns);
        self.log[start..]
            .iter()
            .map(|r| format!("user: {}\nassistant: {}", r.prompt, r.reply.chars().take(reply_chars).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// What the next catch-up says about an undone turn.
pub const UNDO_NOTE: &str = "(the user undid the changes of the previous turn)";

/// What a turn that the user cancelled says in the next catch-up.
pub const INTERRUPTED_NOTE: &str = "(interrupted by the user before it finished; files may be partly changed)";

/// Longest reply kept in a saved session; the catch-up shrinks replies far below this anyway.
pub const SAVED_REPLY_CHARS: usize = 4_000;

/// Tokens are estimated as characters / 4 everywhere in this module.
const CHARS_PER_TOKEN: usize = 4;

fn char_count(s: &str) -> usize {
    s.chars().count()
}

fn estimate_tokens(s: &str) -> usize {
    char_count(s) / CHARS_PER_TOKEN
}

/// Tool names (lowercase) that change files; their argument is the path.
const EDIT_TOOLS: &[&str] = &["edit", "write", "multiedit", "notebookedit", "apply_patch", "patch", "create"];
/// Tool names (lowercase) that run commands; their argument is the command line.
const COMMAND_TOOLS: &[&str] = &["bash", "shell", "command", "run"];

/// The argument of every summary whose tool name is in `names`, deduplicated in
/// first-seen order. Summaries are `Name args`, and backends differ in the case of
/// the name (`Edit` vs `edit`), so the name is matched case-insensitively.
fn args_of_tools(tools: &[String], names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for summary in tools {
        let summary = summary.trim();
        let (name, rest) = summary.split_once(char::is_whitespace).unwrap_or((summary, ""));
        let rest = rest.trim();
        if rest.is_empty() || !names.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        if !out.iter().any(|seen| seen == rest) {
            out.push(rest.to_string());
        }
    }
    out
}

/// Files the tool summaries say were written or edited.
pub fn files_touched(tools: &[String]) -> Vec<String> {
    args_of_tools(tools, EDIT_TOOLS)
}

/// Commands the tool summaries say were run.
pub fn commands_run(tools: &[String]) -> Vec<String> {
    args_of_tools(tools, COMMAND_TOOLS)
}

/// How much of the catch-up to keep. One level per step of shrinking; the order of
/// [`LEVELS`] is the order things are given up: older replies first, then older
/// turns' details, and only at the very end the length of prompts.
struct Level {
    /// Reply chars kept for the last missed turn (the one the user is following up).
    last_reply: usize,
    /// Reply chars kept for earlier turns; 0 drops the reply line.
    older_reply: usize,
    /// Whether earlier turns still list files and commands.
    older_details: bool,
    /// Last-resort cut for every prompt; `None` keeps prompts verbatim.
    prompt_cap: Option<usize>,
    git_lines: usize,
}

const LEVELS: &[Level] = &[
    Level { last_reply: 1500, older_reply: 300, older_details: true, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: true, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: false, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 300, older_reply: 0, older_details: false, prompt_cap: Some(400), git_lines: 10 },
];

/// Who the receiving assistant is: `provider/model`, or just the provider when the
/// model is not known.
fn identity(p: ProviderId, model: Option<&str>) -> String {
    match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!("{p}/{m}"),
        None => p.to_string(),
    }
}

/// The block's header. It must not read as the whole history: the receiving model
/// lived through its own turns and keeps them in its own memory, so it is told that
/// only turns that ran elsewhere are listed.
fn header(you: &str) -> String {
    format!(
        "You are: {you}\n\
         Catch-up: below are ONLY the turns of this conversation that ran on other assistants while you were away \
         (each turn header names the assistant that ran it, as provider/model). \
         Anything you did yourself earlier in this conversation is already in your own memory and is NOT repeated here, \
         so this list is not the whole history. \
         The project folder is shared, so files may have changed since you last looked: re-read a file before editing it. \
         The user's current message follows the line \"Now the user says:\"."
    )
}

/// `s` cut to `n` chars, with an ellipsis when something was removed.
fn truncate_chars(s: &str, n: usize) -> String {
    if char_count(s) <= n {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(n).collect();
    cut.push('…');
    cut
}

/// The first `max` lines of `text`, saying how many were left out.
fn truncate_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.len() <= max {
        return lines.join("\n");
    }
    format!("{}\n... ({} more lines)", lines[..max].join("\n"), lines.len() - max)
}

/// One missed turn. `is_last` turns get the long reply budget and always keep their details.
fn render_turn(index: usize, total: usize, rec: &TurnRecord, is_last: bool, level: &Level) -> String {
    let marker = if rec.ok { "ok" } else { "FAILED" };
    let mut out = format!("[turn {index} of {total}: {}/{}, {marker}]\n", rec.provider, rec.model);
    let prompt = match level.prompt_cap {
        Some(cap) => truncate_chars(&rec.prompt, cap),
        None => rec.prompt.clone(),
    };
    out.push_str(&format!("user: {prompt}\n"));
    let reply_chars = if is_last { level.last_reply } else { level.older_reply };
    if reply_chars > 0 && !rec.reply.trim().is_empty() {
        out.push_str(&format!("assistant: {}\n", truncate_chars(rec.reply.trim(), reply_chars)));
    }
    if is_last || level.older_details {
        for (label, items) in [("files touched", files_touched(&rec.tools)), ("commands run", commands_run(&rec.tools))] {
            if !items.is_empty() {
                out.push_str(&format!("{label}: {}\n", items.join(", ")));
            }
        }
    }
    out
}

/// The whole catch-up block at one level of detail. Pure, so each shrink step is
/// just another call.
fn render(missed: &[TurnRecord], you: &str, git_status: &str, level: &Level) -> String {
    let total = missed.len();
    let mut out = format!("{}\n\n", header(you));
    for (i, rec) in missed.iter().enumerate() {
        out.push_str(&render_turn(i + 1, total, rec, i + 1 == total, level));
        out.push('\n');
    }
    let git = truncate_lines(git_status, level.git_lines);
    let git = if git.is_empty() { "(no changes)".to_string() } else { git };
    out.push_str(&format!("git status:\n{git}\n\nNow the user says:\n"));
    out
}

// ---------------------------------------------------------------------- saved sessions

impl Sessions {
    /// The session as JSON for saving: CLI session ids, how far each provider has seen,
    /// and the turn log with every reply cut to [`SAVED_REPLY_CHARS`].
    pub fn to_json(&self) -> Value {
        let ids: serde_json::Map<String, Value> = self.ids.iter().map(|(p, id)| (p.as_str().into(), json!(id))).collect();
        let seen: serde_json::Map<String, Value> = self.seen.iter().map(|(p, n)| (p.as_str().into(), json!(n))).collect();
        let log: Vec<Value> = self
            .log
            .iter()
            .map(|r| {
                json!({
                    "prompt": r.prompt,
                    "provider": r.provider.as_str(),
                    "model": r.model,
                    "reply": truncate_chars(&r.reply, SAVED_REPLY_CHARS),
                    "tools": r.tools,
                    "ok": r.ok,
                })
            })
            .collect();
        let mut out = json!({ "ids": ids, "seen": seen, "log": log });
        if let Some(name) = &self.name {
            out["name"] = json!(name);
        }
        out
    }

    /// Read what [`Sessions::to_json`] wrote. Tolerant: a field that is missing or the
    /// wrong type is skipped, and an entry that cannot be read is dropped, so a damaged
    /// file gives a shorter session rather than an error.
    pub fn from_json(v: &Value) -> Self {
        let provider = |s: &str| s.parse::<ProviderId>().ok();
        let mut out = Self::default();
        out.set_name(v.get("name").and_then(Value::as_str));
        if let Some(entries) = v.get("log").and_then(Value::as_array) {
            for e in entries {
                let text = |k: &str| e.get(k).and_then(Value::as_str);
                let (Some(prompt), Some(p)) = (text("prompt"), text("provider").and_then(provider)) else { continue };
                out.log.push(TurnRecord {
                    prompt: prompt.to_string(),
                    provider: p,
                    model: text("model").unwrap_or_default().to_string(),
                    reply: text("reply").unwrap_or_default().to_string(),
                    tools: e
                        .get("tools")
                        .and_then(Value::as_array)
                        .map(|t| t.iter().filter_map(Value::as_str).map(String::from).collect())
                        .unwrap_or_default(),
                    ok: e.get("ok").and_then(Value::as_bool).unwrap_or(false),
                });
            }
        }
        if let Some(ids) = v.get("ids").and_then(Value::as_object) {
            for (k, id) in ids {
                if let (Some(p), Some(id)) = (provider(k), id.as_str().filter(|s| !s.is_empty())) {
                    out.ids.insert(p, id.to_string());
                }
            }
        }
        if let Some(seen) = v.get("seen").and_then(Value::as_object) {
            for (k, n) in seen {
                if let (Some(p), Some(n)) = (provider(k), n.as_u64()) {
                    out.seen.insert(p, (n as usize).min(out.log.len()));
                }
            }
        }
        out
    }
}

/// A session saved to disk is ignored once it is older than this.
pub const SAVED_MAX_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// What is stored for one project: the conversation plus where routing stood.
#[derive(Debug, Clone)]
pub struct SavedSession {
    pub project: String,
    pub saved_ms: u64,
    pub sessions: Sessions,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

/// What the UI shows to offer resuming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub turns: usize,
    pub last_used_ms: u64,
    pub providers: Vec<ProviderId>,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    /// Persistence is switched off for this engine.
    Disabled,
    /// Nothing saved for this project (or it is older than 14 days, or for another project).
    NotFound,
    /// The file is not a saved session.
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("saving sessions is switched off"),
            Self::NotFound => f.write_str("no earlier saved session for this folder (the last 10 are kept, for 14 days)"),
            Self::Corrupt(m) => write!(f, "the saved session is unreadable: {m}"),
            Self::Io(m) => write!(f, "could not read the saved session: {m}"),
        }
    }
}

impl std::error::Error for ResumeError {}

impl SavedSession {
    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "project": self.project,
            "saved_ms": self.saved_ms,
            "current": self.current.as_ref().map(|(p, m)| json!({ "provider": p.as_str(), "model": m })),
            "sessions": self.sessions.to_json(),
        })
    }

    /// `Err` only when the value is not a saved session at all (no project or time).
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let project = v.get("project").and_then(Value::as_str).ok_or("no project")?.to_string();
        let saved_ms = v.get("saved_ms").and_then(Value::as_u64).ok_or("no save time")?;
        let current = v.get("current").and_then(|c| {
            let provider = c.get("provider")?.as_str()?.parse::<ProviderId>().ok()?;
            Some((provider, c.get("model")?.as_str()?.to_string()))
        });
        let sessions = Sessions::from_json(v.get("sessions").unwrap_or(&Value::Null));
        Ok(Self { project, saved_ms, sessions, current })
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            turns: self.sessions.turns(),
            last_used_ms: self.saved_ms,
            providers: self.sessions.providers_used(),
            current: self.current.clone(),
        }
    }

    /// Is this too old to offer at time `now_ms`?
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.saved_ms) > SAVED_MAX_AGE_MS
    }
}

/// One saved session as the `/resume` list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub info: SessionInfo,
    /// What the person first asked.
    pub title: String,
}

/// The project folder as it is keyed on disk: canonical when it can be.
pub fn project_key(dir: &Path) -> String {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().into_owned()
}

/// FNV-1a, 64 bit: a hash that is the same on every machine and every Rust version,
/// which `DefaultHasher` does not promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// How many saved sessions are kept per project; the oldest go on save.
pub const SAVED_KEEP: usize = 10;

fn project_hash(project_dir: &Path) -> String {
    format!("{:016x}", fnv1a(project_key(project_dir).as_bytes()))
}

/// `<state_dir>/sessions/<hash of the canonical project folder>/`: one file per session.
pub fn project_sessions_dir(state_dir: &Path, project_dir: &Path) -> PathBuf {
    state_dir.join("sessions").join(project_hash(project_dir))
}

/// The single file older versions kept per project, `<state_dir>/sessions/<hash>.json`.
/// It is still found (and resumed) like any session of the project.
pub fn legacy_session_file(state_dir: &Path, project_dir: &Path) -> PathBuf {
    state_dir.join("sessions").join(format!("{}.json", project_hash(project_dir)))
}

/// A file for a session that starts now: `<project dir>/<now_ms>-<pid>.json`. A name that
/// is taken (two sessions in one millisecond of one process) moves on a millisecond, so a
/// new session never lands on, and overwrites, an older one.
pub fn new_session_file(state_dir: &Path, project_dir: &Path, now_ms: u64) -> PathBuf {
    let dir = project_sessions_dir(state_dir, project_dir);
    let pid = std::process::id();
    (now_ms..).map(|ms| dir.join(format!("{ms}-{pid}.json"))).find(|p| !p.exists()).expect("an unbounded range")
}

/// Write `text` to `path` through a temp file and a rename, so a crash never leaves half a
/// file; owner-only, because prompts can be private.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let written = opts.open(&tmp).and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()));
    match written.and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Read one session file. One that is not for `project_dir` (a hash collision) is
/// [`ResumeError::NotFound`].
fn read_session(path: &Path, project_dir: &Path) -> Result<SavedSession, ResumeError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ResumeError::NotFound),
        Err(e) => return Err(ResumeError::Io(e.to_string())),
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| ResumeError::Corrupt(e.to_string()))?;
    let saved = SavedSession::from_json(&value).map_err(ResumeError::Corrupt)?;
    if saved.project != project_key(project_dir) {
        return Err(ResumeError::NotFound);
    }
    Ok(saved)
}

/// Every session file of the project: those in its folder, and the old single file.
fn session_files(state_dir: &Path, project_dir: &Path) -> Vec<PathBuf> {
    let listed = std::fs::read_dir(project_sessions_dir(state_dir, project_dir))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"));
    let legacy = legacy_session_file(state_dir, project_dir);
    listed.chain(legacy.exists().then_some(legacy)).collect()
}

/// The most recently saved session of `project_dir` other than `current` (the file the
/// running session writes to). Sessions older than 14 days and files of other projects do
/// not count. `NotFound` when there is none; a damaged file is skipped unless it is all
/// there is, and then it is reported.
pub fn load_latest(
    state_dir: &Path,
    project_dir: &Path,
    now_ms: u64,
    current: Option<&Path>,
) -> Result<(PathBuf, SavedSession), ResumeError> {
    let mut best: Option<(PathBuf, SavedSession)> = None;
    let mut broken: Option<ResumeError> = None;
    for path in session_files(state_dir, project_dir).into_iter().filter(|p| Some(p.as_path()) != current) {
        match read_session(&path, project_dir) {
            Ok(s) if s.expired(now_ms) => {}
            // Equal times (a coarse clock): the later file name is the newer session.
            Ok(s) if best.as_ref().is_none_or(|(p, b)| (s.saved_ms, &path) > (b.saved_ms, p)) => best = Some((path, s)),
            Ok(_) | Err(ResumeError::NotFound) => {}
            Err(e) => broken = Some(e),
        }
    }
    best.ok_or(broken.unwrap_or(ResumeError::NotFound))
}

/// Every usable saved session of `project_dir` other than `current`, newest first: the
/// list `/resume` offers. Expired, empty, damaged and other projects' files are left out.
pub fn list_sessions(state_dir: &Path, project_dir: &Path, now_ms: u64, current: Option<&Path>) -> Vec<SessionEntry> {
    let mut found: Vec<(u64, SessionEntry)> = session_files(state_dir, project_dir)
        .into_iter()
        .filter(|p| Some(p.as_path()) != current)
        .filter_map(|path| {
            let saved = read_session(&path, project_dir).ok().filter(|s| !s.expired(now_ms) && s.sessions.turns() > 0)?;
            let title = saved.sessions.title().unwrap_or_else(|| "(no message)".to_string());
            Some((saved.saved_ms, SessionEntry { info: saved.info(), title, path }))
        })
        .collect();
    // Equal times (a coarse clock): the later file name is the newer session.
    found.sort_by(|(a, x), (b, y)| (b, &y.path).cmp(&(a, &x.path)));
    found.into_iter().map(|(_, e)| e).collect()
}

/// Load one session file of the project, as chosen from [`list_sessions`].
pub fn load_path(project_dir: &Path, path: &Path) -> Result<SavedSession, ResumeError> {
    read_session(path, project_dir)
}

/// Keep at most [`SAVED_KEEP`] sessions of the project: delete the oldest, never `current`.
/// A file that cannot be read counts as the oldest. Cheap while under the cap (a listing).
pub fn prune(state_dir: &Path, project_dir: &Path, current: &Path) {
    let files = session_files(state_dir, project_dir);
    if files.len() <= SAVED_KEEP {
        return;
    }
    let mut aged: Vec<(u64, PathBuf)> =
        files.into_iter().map(|p| (read_session(&p, project_dir).map_or(0, |s| s.saved_ms), p)).collect();
    aged.sort();
    let surplus = aged.len() - SAVED_KEEP;
    for (_, path) in aged.into_iter().filter(|(_, p)| p != current).take(surplus) {
        let _ = std::fs::remove_file(path);
    }
}

/// What there is to resume for `project_dir` in `state_dir` (its most recent session), if
/// anything usable.
pub fn saved_session_info_in(state_dir: &Path, project_dir: &Path, now_ms: u64) -> Option<SessionInfo> {
    load_latest(state_dir, project_dir, now_ms, None).ok().map(|(_, s)| s.info()).filter(|i| i.turns > 0)
}

/// [`saved_session_info_in`] for the default state directory and the real clock, for
/// the UI to offer resuming at startup.
pub fn saved_session_info(project_dir: &Path) -> Option<SessionInfo> {
    saved_session_info_in(&crate::log::state_dir()?, project_dir, crate::log::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(provider: ProviderId, prompt: &str, reply: &str, tools: &[&str], ok: bool) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            provider,
            model: "m1".into(),
            reply: reply.into(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            ok,
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn nothing_missed_gives_none() {
        let mut s = Sessions::new();
        assert_eq!(s.catch_up(ProviderId::Claude, "", 1000), None);
        s.record(rec(ProviderId::Claude, "hi", "hello", &[], true));
        assert_eq!(s.catch_up(ProviderId::Claude, "", 1000), None);
    }

    #[test]
    fn one_missed_turn_has_prompt_file_and_git_status() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "Fix the parser\nin src/a.rs, please", "Done.", &["Edit src/a.rs", "Bash cargo test"], true));
        let text = s.catch_up(ProviderId::Codex, " M src/a.rs", 1000).unwrap();
        assert!(text.contains("Fix the parser\nin src/a.rs, please"));
        assert!(text.contains("files touched: src/a.rs"));
        assert!(text.contains("commands run: cargo test"));
        assert!(text.contains("Done."));
        assert!(text.contains("git status:\n M src/a.rs"));
        assert!(text.contains("ok]"));
        assert!(text.trim_end().ends_with("Now the user says:"));
    }

    #[test]
    fn header_says_only_other_assistants_turns_are_listed_and_never_claims_full_history() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "rename the word", "renamed", &["Edit src/a.rs"], true));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("ONLY the turns"), "{text}");
        assert!(text.contains("other assistants while you were away"));
        assert!(text.contains("provider/model"));
        assert!(text.contains("already in your own memory and is NOT repeated here"));
        assert!(text.contains("shared") && text.contains("re-read"));
        assert!(text.contains("\"Now the user says:\""));
        for old in ["done by other assistants", "other assistants did the work", "The work so far", "You did not see it"] {
            assert!(!text.contains(old), "old phrase {old:?} is back");
        }
    }

    #[test]
    fn header_names_the_receiving_identity() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        let with = s.catch_up_for(ProviderId::Codex, "gpt-6-luna", "", 1000).unwrap();
        assert!(with.starts_with("You are: codex/gpt-6-luna\n"), "{with}");
        // The wrapper has no model, so only the provider is named.
        let without = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(without.starts_with("You are: codex\n"), "{without}");
        // Turn headers still name who ran each turn.
        assert!(with.contains("[turn 1 of 1: claude/m1, ok]"));
        // The identity survives the tightest budget.
        let tiny = s.catch_up_for(ProviderId::Codex, "gpt-6-luna", "", 1).unwrap();
        assert!(tiny.contains("You are: codex/gpt-6-luna"));
    }

    #[test]
    fn failed_turn_is_listed_and_marked() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "try the build", "", &["Bash cargo build"], false));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("try the build"));
        assert!(text.contains("FAILED"));
        assert!(text.contains("commands run: cargo build"));
    }

    #[test]
    fn provider_that_ran_the_turn_never_misses_it() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "one", "r1", &[], true));
        s.record(rec(ProviderId::Codex, "two", "r2", &[], true));
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.missed_turns(ProviderId::Claude), 1);
        assert_eq!(s.missed_turns(ProviderId::Gemini), 2);
    }

    #[test]
    fn mark_seen_clears_missed_until_the_next_foreign_turn() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "one", "r1", &[], true));
        assert_eq!(s.missed_turns(ProviderId::Codex), 1);
        s.mark_seen(ProviderId::Codex);
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.catch_up(ProviderId::Codex, "", 1000), None);
        s.record(rec(ProviderId::Claude, "two", "r2", &[], true));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("two") && !text.contains("user: one"));
        assert!(text.contains("[turn 1 of 1"));
    }

    fn long_session() -> Sessions {
        let mut s = Sessions::new();
        for i in 0..4 {
            let reply = format!("REPLY{i}{}", i.to_string().repeat(1200));
            s.record(rec(ProviderId::Claude, &format!("PROMPT{i} please"), &reply, &[&format!("Edit src/f{i}.rs")], true));
        }
        s
    }

    #[test]
    fn full_budget_keeps_older_replies_short_and_last_reply_long() {
        let text = long_session().catch_up(ProviderId::Codex, "", 100_000).unwrap();
        assert!(text.contains("REPLY0") && text.contains("REPLY1"));
        assert!(text.contains("files touched: src/f0.rs"));
        // Earlier replies are cut to ~300 chars, the last one keeps up to 1500.
        assert!(!text.contains(&"0".repeat(400)));
        assert!(text.contains(&"0".repeat(290)));
        assert!(text.contains(&"3".repeat(1100)));
    }

    #[test]
    fn shrinking_budget_drops_old_replies_before_details_before_prompt_length() {
        let s = long_session();
        let full = s.catch_up(ProviderId::Codex, "", 100_000).unwrap();
        // Just under the full size: older replies go, details stay.
        let no_old_replies = s.catch_up(ProviderId::Codex, "", estimate_tokens(&full) - 1).unwrap();
        assert!(!no_old_replies.contains("REPLY0") && !no_old_replies.contains("REPLY2"));
        assert!(no_old_replies.contains("REPLY3"));
        assert!(no_old_replies.contains("files touched: src/f0.rs"));
        // Tighter: older details go too.
        let tight = s.catch_up(ProviderId::Codex, "", estimate_tokens(&no_old_replies) - 1).unwrap();
        assert!(!tight.contains("src/f0.rs") && !tight.contains("src/f2.rs"));
        assert!(tight.contains("src/f3.rs") && tight.contains("REPLY3"));
        for i in 0..4 {
            assert!(tight.contains(&format!("PROMPT{i} please")));
        }
        assert!(estimate_tokens(&tight) < estimate_tokens(&no_old_replies));
    }

    #[test]
    fn prompts_survive_an_impossible_budget_and_only_then_get_cut() {
        let mut s = Sessions::new();
        let long_prompt = format!("START {} END", "p".repeat(2000));
        s.record(rec(ProviderId::Claude, &long_prompt, "r", &[], true));
        s.record(rec(ProviderId::Claude, "second prompt", "r", &[], true));
        let fits = s.catch_up(ProviderId::Codex, "", 100_000).unwrap();
        assert!(fits.contains(&long_prompt), "verbatim when it fits");
        let tiny = s.catch_up(ProviderId::Codex, "", 1).unwrap();
        assert!(tiny.contains("START ") && tiny.contains("second prompt"));
        assert!(!tiny.contains("END"), "last resort cuts to 400 chars");
        assert!(tiny.contains("Now the user says:"));
    }

    #[test]
    fn git_status_is_cut_to_forty_lines() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        let git: String = (0..60).map(|i| format!(" M file{i}.rs\n")).collect();
        let text = s.catch_up(ProviderId::Codex, &git, 100_000).unwrap();
        assert!(text.contains("file39.rs") && !text.contains("file40.rs"));
        assert!(text.contains("(20 more lines)"));
    }

    #[test]
    fn reset_forgets_everything() {
        let mut s = Sessions::new();
        s.set_session_id(ProviderId::Claude, "abc".into());
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        assert_eq!(s.session_id(ProviderId::Claude), Some("abc"));
        s.reset();
        assert_eq!(s.session_id(ProviderId::Claude), None);
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.context_tokens_estimate(), 0);
        assert_eq!(s.recent_context(3, 100), "");
    }

    #[test]
    fn context_estimate_is_chars_over_four() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, &"a".repeat(40), &"b".repeat(60), &["Edit x"], true));
        assert_eq!(s.context_tokens_estimate(), (40 + 60 + 6) / 4);
    }

    #[test]
    fn recent_context_format_newest_last_and_truncated() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "q1", "a1", &[], true));
        s.record(rec(ProviderId::Codex, "q2", "abcdefgh", &[], true));
        s.record(rec(ProviderId::Claude, "q3", "a3", &[], true));
        assert_eq!(s.recent_context(2, 4), "user: q2\nassistant: abcd\nuser: q3\nassistant: a3");
        assert_eq!(s.recent_context(10, 100).matches("user: ").count(), 3);
    }

    #[test]
    fn files_touched_and_commands_run_classify_by_tool_name() {
        let tools = strings(&[
            "Edit src/a.rs",
            "Write /abs/b.md",
            "MultiEdit f",
            "NotebookEdit n",
            "Bash cargo test",
            "shell cargo build",
            "edit src/c.rs",
            "apply_patch *** Begin Patch",
            "Edit src/a.rs",
            "Read src/z.rs",
            "Bash cargo test",
            "Bash",
        ]);
        assert_eq!(
            files_touched(&tools),
            strings(&["src/a.rs", "/abs/b.md", "f", "n", "src/c.rs", "*** Begin Patch"])
        );
        assert_eq!(commands_run(&tools), strings(&["cargo test", "cargo build"]));
    }

    #[test]
    fn a_failed_turn_keeps_a_provider_that_was_behind_owed_its_catch_up() {
        let rec = |p, ok: bool| TurnRecord { prompt: "q".into(), provider: p, model: "m".into(), reply: "r".into(), tools: vec![], ok };
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, true));
        s.record(rec(ProviderId::Claude, true));
        // codex is behind (missed 2 turns) and its attempt fails: still owed all 3
        s.record_failed(rec(ProviderId::Codex, false));
        assert_eq!(s.missed_turns(ProviderId::Codex), 3);
        // a provider that was current keeps its place when its own turn fails
        s.mark_seen(ProviderId::Claude);
        s.record_failed(rec(ProviderId::Claude, false));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0);
    }

    #[test]
    fn sessions_round_trip_through_json_and_cap_long_replies() {
        let mut s = Sessions::new();
        s.set_session_id(ProviderId::Claude, "abc".into());
        s.set_session_id(ProviderId::Codex, "xyz".into());
        s.record(rec(ProviderId::Claude, "one", "r1", &["Edit src/a.rs"], true));
        s.record(rec(ProviderId::Codex, "two", &"r".repeat(SAVED_REPLY_CHARS + 500), &[], false));
        let back = Sessions::from_json(&s.to_json());
        assert_eq!(back.session_id(ProviderId::Claude), Some("abc"));
        assert_eq!(back.session_id(ProviderId::Codex), Some("xyz"));
        assert_eq!(back.turns(), 2);
        assert_eq!(back.missed_turns(ProviderId::Codex), 0);
        assert_eq!(back.missed_turns(ProviderId::Claude), 1, "who has seen what survives");
        assert_eq!(back.log[0], s.log[0]);
        assert!(!back.log[1].ok);
        assert_eq!(back.log[1].reply.chars().count(), SAVED_REPLY_CHARS + 1, "capped, with an ellipsis");
        assert_eq!(back.providers_used(), [ProviderId::Claude, ProviderId::Codex]);
        assert_eq!(back.last_answered(), Some(("one", "r1")));
    }

    #[test]
    fn a_name_is_saved_and_read_back_and_titles_the_session() {
        let mut s = Sessions::new();
        assert_eq!(s.title(), None);
        s.set_name(Some("  parser   fix "));
        assert_eq!(s.name(), Some("parser fix"));
        let back = Sessions::from_json(&s.to_json());
        assert_eq!(back.name(), Some("parser fix"));
        assert_eq!(back.title().as_deref(), Some("parser fix"));
        s.set_name(Some("   "));
        assert_eq!(s.name(), None, "a blank name takes the name away");
        assert!(s.to_json().get("name").is_none());
    }

    #[test]
    fn from_json_survives_garbage_and_missing_fields() {
        for v in [json!(null), json!(7), json!("x"), json!([]), json!({}), json!({ "log": "no", "ids": 3, "seen": [1] })] {
            let s = Sessions::from_json(&v);
            assert_eq!((s.turns(), s.session_id(ProviderId::Claude)), (0, None), "{v}");
        }
        let v = json!({
            "log": [
                { "prompt": "ok", "provider": "claude" },
                { "prompt": "unknown provider", "provider": "nope" },
                { "provider": "claude" },
                7,
                { "prompt": "tools are junk", "provider": "codex", "tools": [1, "Edit a", null], "ok": "yes", "reply": 5 },
            ],
            "ids": { "claude": "", "codex": "c1", "nope": "x", "grok": 5 },
            "seen": { "claude": 99, "codex": "many", "nope": 1 },
        });
        let s = Sessions::from_json(&v);
        assert_eq!(s.turns(), 2);
        assert_eq!(s.log[1].tools, ["Edit a"]);
        assert!(!s.log[1].ok && s.log[1].reply.is_empty());
        assert_eq!(s.session_id(ProviderId::Claude), None);
        assert_eq!(s.session_id(ProviderId::Codex), Some("c1"));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0, "a seen count past the end is clamped");
    }

    #[test]
    fn an_undo_is_told_to_every_provider_including_the_one_that_ran_the_turn() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "change it", "done", &["Edit a.rs"], true));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0);
        s.record_undo();
        assert_eq!(s.missed_turns(ProviderId::Claude), 1);
        let text = s.catch_up(ProviderId::Claude, "", 1000).unwrap();
        assert!(text.contains("the user undid the changes of the previous turn"), "{text}");
        assert!(text.contains("FAILED"));
        assert_eq!(s.last_answered(), Some(("change it", "done")), "an undo note is not an answer");
    }

    struct TempState(PathBuf);

    impl TempState {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("jev-sessions-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const NOW: u64 = 1_760_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn saved_for(dir: &Path, saved_ms: u64) -> SavedSession {
        let mut sessions = Sessions::new();
        sessions.set_session_id(ProviderId::Claude, "s1".into());
        sessions.record(rec(ProviderId::Claude, "hi", "hello", &[], true));
        SavedSession { project: project_key(dir), saved_ms, sessions, current: Some((ProviderId::Claude, "m1".into())) }
    }

    fn store(state: &Path, dir: &Path, saved: &SavedSession) -> PathBuf {
        let path = new_session_file(state, dir, saved.saved_ms);
        write_atomic(&path, &saved.to_json().to_string()).unwrap();
        path
    }

    #[test]
    fn a_saved_session_round_trips_through_its_file() {
        let (state, project) = (TempState::new("rt-state"), TempState::new("rt-proj"));
        let saved = saved_for(&project.0, NOW - 1000);
        store(&state.0, &project.0, &saved);
        let (file, back) = load_latest(&state.0, &project.0, NOW, None).unwrap();
        assert_eq!((back.saved_ms, back.project.as_str()), (NOW - 1000, saved.project.as_str()));
        assert_eq!(back.current, Some((ProviderId::Claude, "m1".into())));
        assert_eq!(back.sessions.session_id(ProviderId::Claude), Some("s1"));
        let info = saved_session_info_in(&state.0, &project.0, NOW).unwrap();
        assert_eq!(info, SessionInfo { turns: 1, last_used_ms: NOW - 1000, providers: vec![ProviderId::Claude], current: back.current });
        // Written atomically and privately: no temp file left, owner-only on unix.
        let names: Vec<_> = std::fs::read_dir(project_sessions_dir(&state.0, &project.0)).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn a_garbage_or_foreign_file_is_an_error_not_a_session() {
        let (state, project) = (TempState::new("bad-state"), TempState::new("bad-proj"));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
        let file = new_session_file(&state.0, &project.0, NOW);
        for junk in ["not json at all", "[1,2,3]", "{}", r#"{"project":"x"}"#] {
            write_atomic(&file, junk).unwrap();
            assert!(matches!(load_latest(&state.0, &project.0, NOW, None), Err(ResumeError::Corrupt(_))), "{junk}");
            assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
        }
        // A file that says it is for another project is not ours.
        let mut other = saved_for(&project.0, NOW);
        other.project = "/somewhere/else".into();
        write_atomic(&file, &other.to_json().to_string()).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
    }

    #[test]
    fn sessions_older_than_fourteen_days_are_ignored() {
        let (state, project) = (TempState::new("age-state"), TempState::new("age-proj"));
        let file = store(&state.0, &project.0, &saved_for(&project.0, NOW - 13 * DAY));
        assert!(saved_session_info_in(&state.0, &project.0, NOW).is_some());
        write_atomic(&file, &saved_for(&project.0, NOW - 15 * DAY).to_json().to_string()).unwrap();
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap_err(), ResumeError::NotFound);
    }

    #[test]
    fn the_most_recent_session_other_than_the_current_one_is_found() {
        let (state, project) = (TempState::new("latest-state"), TempState::new("latest-proj"));
        let old = store(&state.0, &project.0, &saved_for(&project.0, NOW - 3 * DAY));
        let newer = store(&state.0, &project.0, &saved_for(&project.0, NOW - DAY));
        let expired = store(&state.0, &project.0, &saved_for(&project.0, NOW - 20 * DAY));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, newer);
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap().0, old, "the running session is skipped");
        std::fs::remove_file(&old).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap_err(), ResumeError::NotFound);
        assert!(expired.exists(), "an expired file is ignored, not deleted by a lookup");
    }

    #[test]
    fn the_old_single_file_is_still_found_and_ranks_by_its_time() {
        let (state, project) = (TempState::new("legacy-state"), TempState::new("legacy-proj"));
        let legacy = legacy_session_file(&state.0, &project.0);
        write_atomic(&legacy, &saved_for(&project.0, NOW - DAY).to_json().to_string()).unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, legacy);
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW).unwrap().turns, 1);
        let newer = store(&state.0, &project.0, &saved_for(&project.0, NOW - 1000));
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, newer);
        assert_eq!(load_latest(&state.0, &project.0, NOW, Some(&newer)).unwrap().0, legacy);
    }

    #[test]
    fn a_damaged_file_is_skipped_when_a_good_one_exists() {
        let (state, project) = (TempState::new("skip-state"), TempState::new("skip-proj"));
        let good = store(&state.0, &project.0, &saved_for(&project.0, NOW - DAY));
        write_atomic(&project_sessions_dir(&state.0, &project.0).join("1-1.json"), "{ broken").unwrap();
        assert_eq!(load_latest(&state.0, &project.0, NOW, None).unwrap().0, good);
    }

    #[test]
    fn new_session_files_never_collide() {
        let (state, project) = (TempState::new("collide-state"), TempState::new("collide-proj"));
        let first = store(&state.0, &project.0, &saved_for(&project.0, NOW));
        let second = new_session_file(&state.0, &project.0, NOW);
        assert_ne!(first, second, "a session started in the same millisecond must not take over the first one's file");
        assert_eq!(second.parent(), Some(project_sessions_dir(&state.0, &project.0).as_path()));
    }

    #[test]
    fn only_the_newest_ten_sessions_are_kept_and_the_current_one_never_goes() {
        let (state, project) = (TempState::new("prune-state"), TempState::new("prune-proj"));
        let files: Vec<PathBuf> = (0..13).map(|i| store(&state.0, &project.0, &saved_for(&project.0, NOW - (20 - i) * 1000))).collect();
        prune(&state.0, &project.0, &files[12]);
        let left = session_files(&state.0, &project.0);
        assert_eq!(left.len(), SAVED_KEEP);
        assert!(files[..3].iter().all(|f| !f.exists()), "the three oldest are gone");
        assert!(files[3..].iter().all(|f| f.exists()));
        // The current session is spared even when it is the oldest.
        let oldest = store(&state.0, &project.0, &saved_for(&project.0, NOW - 100_000));
        prune(&state.0, &project.0, &oldest);
        assert!(oldest.exists());
        assert_eq!(session_files(&state.0, &project.0).len(), SAVED_KEEP);
        assert!(!files[3].exists(), "the next oldest went instead");
    }

    #[test]
    fn each_project_has_its_own_file_and_the_name_is_stable() {
        let state = TempState::new("iso-state");
        let (a, b) = (TempState::new("iso-a"), TempState::new("iso-b"));
        assert_ne!(project_sessions_dir(&state.0, &a.0), project_sessions_dir(&state.0, &b.0));
        // A path spelled differently but the same folder keys the same file.
        let roundabout = a.0.join("..").join(a.0.file_name().unwrap());
        assert_eq!(project_sessions_dir(&state.0, &a.0), project_sessions_dir(&state.0, &roundabout));
        store(&state.0, &a.0, &saved_for(&a.0, NOW));
        assert!(saved_session_info_in(&state.0, &a.0, NOW).is_some());
        assert_eq!(saved_session_info_in(&state.0, &b.0, NOW), None);
        // FNV-1a test vector, so the file names never drift between releases.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_saved_session_without_turns_is_not_offered() {
        let (state, project) = (TempState::new("empty-state"), TempState::new("empty-proj"));
        let mut saved = saved_for(&project.0, NOW);
        saved.sessions = Sessions::new();
        store(&state.0, &project.0, &saved);
        assert_eq!(saved_session_info_in(&state.0, &project.0, NOW), None);
    }
}
