//! Codex backend: `codex exec --json`. One JSON event per stdout line; a turn is
//! `thread.started`, any number of `item.*` events, then `turn.completed` or
//! `turn.failed`. Unlike Claude's `result` line, `turn.completed` carries neither the
//! answer nor the thread id, so the backend remembers both while the turn streams.
//!
//! Availability: with a missing or expired login codex does not fail fast, it retries
//! the 401 for about 35 seconds. An availability probe must therefore ask
//! `codex login status` instead of discovering this by running a turn.

use std::sync::{Mutex, MutexGuard};

use serde_json::{json, Value};
use tokio::process::Command;

use super::claude::tool_summary;
use super::{classify_common, Backend, ErrorKind, Event, Finish, McpAttachment, ProviderId, TurnRequest, Usage};

/// The only sandboxes this backend will ever pass on. Anything else, in particular
/// `danger-full-access`, silently becomes [`DEFAULT_SANDBOX`]: a stray env value must
/// never turn into an unsandboxed agent.
const SAFE_SANDBOXES: [&str; 2] = ["read-only", "workspace-write"];
const DEFAULT_SANDBOX: &str = "workspace-write";

/// What `turn.completed` needs from earlier lines of the same turn.
#[derive(Default)]
struct TurnState {
    thread_id: Option<String>,
    /// The last `agent_message` text: codex's final answer.
    answer: String,
}

pub struct CodexBackend {
    pub bin: String,
    /// `read-only` or `workspace-write`; see [`CodexBackend::sandbox_arg`].
    pub sandbox: String,
    /// Interior mutability because [`Backend::parse_line`] takes `&self`. It is reset by
    /// every `thread.started`, so use one backend per turn: two turns streaming through
    /// the same instance at once would mix their answers.
    state: Mutex<TurnState>,
    /// The orchestration planner's delegate tool, passed as `-c` overrides.
    mcp: Option<McpAttachment>,
}

impl CodexBackend {
    /// Codex with the user's own ChatGPT login. Binary and sandbox come from the
    /// environment so every front end (code, gateway workers) behaves the same.
    pub fn codex() -> Self {
        Self::new(
            std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            std::env::var("CODEX_SANDBOX").unwrap_or_else(|_| DEFAULT_SANDBOX.into()),
        )
    }

    /// Explicit binary and sandbox, for tests and callers that do not read the env.
    pub fn new(bin: impl Into<String>, sandbox: impl Into<String>) -> Self {
        Self { bin: bin.into(), sandbox: sandbox.into(), state: Mutex::new(TurnState::default()), mcp: None }
    }

    /// The configured sandbox if it is one of [`SAFE_SANDBOXES`], else the default.
    pub fn sandbox_arg(&self) -> &str {
        SAFE_SANDBOXES.iter().find(|s| **s == self.sandbox).copied().unwrap_or(DEFAULT_SANDBOX)
    }

    /// A poisoned lock only means another thread panicked mid-update; the state is
    /// plain data and the next `thread.started` resets it, so keep going.
    fn state(&self) -> MutexGuard<'_, TurnState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// New turn: `-C` and `-s` are only accepted by plain `exec`.
    fn fresh_args(&self, cmd: &mut Command, req: &TurnRequest) {
        cmd.args(["exec", "--json", "-C"]).arg(&req.dir).args(["-s", self.sandbox_arg(), "-m", &req.model]);
        cmd.args(self.mcp_overrides());
    }

    /// `-c mcp_servers.<name>.command=... -c mcp_servers.<name>.args=[...]`: Codex has
    /// no per-run config file flag for MCP, but `-c` takes dotted TOML overrides (checked
    /// against `codex exec --help`). A JSON string is a valid TOML basic string, so
    /// `serde_json` does the quoting. UNVERIFIED end to end: whether `codex exec` runs
    /// the server's tools without an approval prompt was not tried (no quota spent).
    fn mcp_overrides(&self) -> Vec<String> {
        let Some(m) = &self.mcp else { return vec![] };
        let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let args: Vec<String> = m.args.iter().map(|a| quote(a)).collect();
        let key = format!("mcp_servers.{}", m.name);
        let command = quote(&m.command.to_string_lossy());
        vec![
            "-c".into(),
            format!("{key}.command={command}"),
            "-c".into(),
            format!("{key}.args=[{}]", args.join(",")),
        ]
    }

    /// Continued turn: `exec resume` has neither `-C` nor `-s`, so the directory comes
    /// from `current_dir` and the sandbox is re-asserted as a config override. The
    /// trailing `-` makes codex read the prompt from stdin.
    fn resume_args(&self, cmd: &mut Command, req: &TurnRequest, thread_id: &str) {
        let sandbox = format!("sandbox_mode=\"{}\"", self.sandbox_arg());
        cmd.args(["exec", "resume", "--json", "-m", &req.model, "-c", &sandbox]);
        cmd.args(self.mcp_overrides());
        cmd.args([thread_id, "-"]);
    }

    /// Messages are remembered (the last one is the answer) and shown; file changes
    /// are shown. `error` items are warnings such as "model metadata not found" and
    /// do not stop the turn, so they are ignored.
    fn completed_events(&self, item: &Value) -> Vec<Event> {
        match item["type"].as_str() {
            Some("agent_message") => {
                let text = item["text"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    return vec![];
                }
                self.state().answer = text.to_string();
                vec![Event::Text(text.to_string())]
            }
            Some("file_change") => file_change_events(item),
            _ => vec![],
        }
    }
}

/// `/usr/bin/bash -lc 'cat a.txt'` is noise; the user wants `cat a.txt`.
fn shell_script(command: &str) -> &str {
    let Some((_, script)) = command.split_once(" -lc ") else { return command };
    ['\'', '"']
        .iter()
        .find_map(|q| script.strip_prefix(*q).and_then(|s| s.strip_suffix(*q)))
        .unwrap_or(script)
}

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Codex's `input_tokens` already includes the cached share, which is the scale
/// [`Usage`] wants, and the turn's last request is not reported separately, so the
/// whole input is the best available context size.
fn usage_of(u: &Value) -> Usage {
    Usage {
        input_tokens: count(u, "input_tokens"),
        cached_input_tokens: count(u, "cached_input_tokens"),
        output_tokens: count(u, "output_tokens"),
        context_tokens: Some(count(u, "input_tokens")),
    }
}

/// One `edit <path>` per changed file. A failed patch changed nothing, so it is not
/// reported as an edit.
fn file_change_events(item: &Value) -> Vec<Event> {
    if item["status"] == "failed" {
        return vec![];
    }
    let changes = item["changes"].as_array().into_iter().flatten();
    changes.filter_map(|c| c["path"].as_str()).map(|p| Event::Tool(tool_summary("edit", &json!({"path": p})))).collect()
}

/// Commands are announced when they start, so the user sees what is running.
fn started_events(item: &Value) -> Vec<Event> {
    match item["type"].as_str() {
        Some("command_execution") => {
            let script = shell_script(item["command"].as_str().unwrap_or(""));
            vec![Event::Tool(tool_summary("shell", &json!({"command": script})))]
        }
        _ => vec![],
    }
}

fn failure_message(ev: &Value) -> String {
    match ev["error"]["message"].as_str() {
        Some(m) if !m.trim().is_empty() => m.to_string(),
        _ => "codex reported an error".to_string(),
    }
}

impl Backend for CodexBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        match &req.session_id {
            Some(id) => self.resume_args(&mut cmd, req, id),
            None => self.fresh_args(&mut cmd, req),
        }
        cmd.current_dir(&req.dir);
        cmd
    }

    fn supports_mcp(&self) -> bool {
        true
    }

    fn attach_mcp(&mut self, mcp: &McpAttachment) {
        self.mcp = Some(mcp.clone());
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("thread.started") => {
                *self.state() = TurnState { thread_id: ev["thread_id"].as_str().map(String::from), answer: String::new() };
                vec![]
            }
            Some("item.started") => started_events(&ev["item"]),
            Some("item.completed") => self.completed_events(&ev["item"]),
            Some("turn.completed") => {
                let state = self.state();
                vec![Event::Finished(Finish {
                    text: state.answer.clone(),
                    session_id: state.thread_id.clone(),
                    usage: usage_of(&ev["usage"]),
                })]
            }
            // Top-level `error` lines are retry chatter ("Reconnecting... 2/5"); only
            // `turn.failed` means codex gave up.
            Some("turn.failed") => vec![Event::Failed { message: failure_message(&ev) }],
            _ => vec![],
        }
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        if let Some(kind) = classify_common(message) {
            return kind;
        }
        let lower = message.to_lowercase();
        if lower.contains("not supported when using codex with a chatgpt account") {
            ErrorKind::NotEntitled
        } else if lower.contains("401") || lower.contains("missing bearer") {
            ErrorKind::Auth
        } else if ["not inside a git", "not a git repo", "requires a git repo"].iter().any(|p| lower.contains(p)) {
            ErrorKind::UnusableHere
        } else {
            ErrorKind::Other
        }
    }
}

/// Models this login can pick, from the text of `~/.codex/models_cache.json`: the
/// slugs whose `visibility` is `list` (the rest are hidden or deprecated). Unreadable
/// or unexpected JSON gives an empty list, which callers treat as "unknown".
pub fn allowed_models_from_cache(json: &str) -> Vec<String> {
    let Ok(cache) = serde_json::from_str::<Value>(json) else { return vec![] };
    let models = cache["models"].as_array().into_iter().flatten();
    models
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| m["slug"].as_str().map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN1: &str = include_str!("../../tests/fixtures/codex/turn1.jsonl");
    const TURN2: &str = include_str!("../../tests/fixtures/codex/turn2_resume.jsonl");
    const ERR_PLAN: &str = include_str!("../../tests/fixtures/codex/err_model_not_on_plan.jsonl");

    fn backend() -> CodexBackend {
        CodexBackend::new("codex", "workspace-write")
    }

    fn parse_all(backend: &CodexBackend, fixture: &str) -> Vec<Event> {
        fixture.lines().flat_map(|l| backend.parse_line(l)).collect()
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest { dir: "/tmp/proj".into(), model: "gpt-x".into(), prompt: "hi".into(), session_id: session.map(String::from) }
    }

    #[test]
    fn first_turn_parses_to_tools_text_and_finish() {
        let events = parse_all(&backend(), TURN1);
        assert_eq!(events.len(), 5, "{events:?}");
        assert_eq!(events[0], Event::Text("I\u{2019}ll read `a.txt`, then append `world` on a new line.".into()));
        assert_eq!(events[1], Event::Tool("shell cat a.txt".into()));
        assert!(matches!(&events[2], Event::Tool(s) if s.starts_with("shell printf ") && s.ends_with(">> a.txt")), "{:?}", events[2]);
        assert_eq!(events[3], Event::Text("hello".into()));
        let Event::Finished(f) = &events[4] else { panic!("{:?}", events[4]) };
        // The LAST agent_message is the answer; the first one was only narration.
        assert_eq!(f.text, "hello");
        assert_eq!(f.session_id.as_deref(), Some("01a1e5a6-9f98-7c21-99f5-6468805d7543"));
        assert_eq!(
            f.usage,
            Usage { input_tokens: 44970, cached_input_tokens: 36864, output_tokens: 159, context_tokens: Some(44970) }
        );
    }

    #[test]
    fn completed_commands_are_not_announced_twice() {
        let tools = parse_all(&backend(), TURN1).into_iter().filter(|e| matches!(e, Event::Tool(_))).count();
        assert_eq!(tools, 2);
    }

    #[test]
    fn resumed_turn_keeps_the_thread_id_and_answers() {
        let events = parse_all(&backend(), TURN2);
        let Some(Event::Finished(f)) = events.last() else { panic!("{events:?}") };
        assert_eq!(f.text, "world");
        assert_eq!(f.session_id.as_deref(), Some("01a1e5a6-9f98-7c21-99f5-6468805d7543"));
        assert_eq!(f.usage.input_tokens, 60251);
        assert_eq!(f.usage.cached_input_tokens, 47616);
        assert_eq!(f.usage.output_tokens, 164);
    }

    #[test]
    fn a_new_thread_does_not_inherit_the_previous_answer() {
        let b = backend();
        parse_all(&b, TURN1);
        let events = b.parse_line(r#"{"type":"thread.started","thread_id":"t2"}"#);
        assert!(events.is_empty());
        let Event::Finished(f) = &b.parse_line(r#"{"type":"turn.completed","usage":{}}"#)[0] else { panic!() };
        assert_eq!((f.text.as_str(), f.session_id.as_deref()), ("", Some("t2")));
    }

    #[test]
    fn model_not_on_plan_fails_only_on_turn_failed() {
        let b = backend();
        let events = parse_all(&b, ERR_PLAN);
        // The warning item and the top-level `error` line produce nothing.
        let message = "The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account.";
        assert_eq!(events, vec![Event::Failed { message: message.into() }]);
        assert_eq!(b.classify_error(message), ErrorKind::NotEntitled);
    }

    #[test]
    fn reconnecting_lines_are_not_failures() {
        let line = r#"{"type":"error","message":"Reconnecting... 2/5 (stream disconnected)"}"#;
        assert!(backend().parse_line(line).is_empty());
    }

    #[test]
    fn turn_failed_without_a_message_still_fails() {
        let evs = backend().parse_line(r#"{"type":"turn.failed"}"#);
        assert_eq!(evs, vec![Event::Failed { message: "codex reported an error".into() }]);
    }

    #[test]
    fn file_changes_are_shown_as_edits_unless_they_failed() {
        let ok = r#"{"type":"item.completed","item":{"type":"file_change","status":"completed","changes":[{"path":"/p/a.rs","kind":"update"},{"path":"/p/b.rs","kind":"add"}]}}"#;
        assert_eq!(backend().parse_line(ok), vec![Event::Tool("edit /p/a.rs".into()), Event::Tool("edit /p/b.rs".into())]);
        let failed = ok.replace("completed\"", "failed\"");
        assert!(backend().parse_line(&failed).is_empty());
    }

    #[test]
    fn junk_and_unknown_lines_are_ignored() {
        let b = backend();
        assert!(b.parse_line("not json").is_empty());
        assert!(b.parse_line("").is_empty());
        assert!(b.parse_line(r#"{"type":"turn.started"}"#).is_empty());
        assert!(b.parse_line(r#"{"type":"item.completed","item":{"type":"reasoning","text":"x"}}"#).is_empty());
    }

    #[test]
    fn shell_wrapper_is_shortened_and_capped() {
        assert_eq!(shell_script("/usr/bin/bash -lc 'cat a.txt'"), "cat a.txt");
        assert_eq!(shell_script("/usr/bin/bash -lc \"echo 'x'\""), "echo 'x'");
        assert_eq!(shell_script("ls -la"), "ls -la");
        let long = format!("/usr/bin/bash -lc '{}'", "x".repeat(300));
        let Event::Tool(s) = &started_events(&json!({"type":"command_execution","command":long}))[0] else { panic!() };
        assert_eq!(s.chars().count(), "shell ".len() + 100);
    }

    #[test]
    fn new_turn_command_sets_dir_sandbox_and_model() {
        let cmd = backend().command(&req(None));
        assert_eq!(args(&cmd), ["exec", "--json", "-C", "/tmp/proj", "-s", "workspace-write", "-m", "gpt-x"]);
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp/proj")));
        assert_eq!(cmd.as_std().get_program(), "codex");
    }

    #[test]
    fn resume_command_reads_the_prompt_from_stdin_and_keeps_the_sandbox() {
        let cmd = backend().command(&req(Some("thread-1")));
        assert_eq!(
            args(&cmd),
            ["exec", "resume", "--json", "-m", "gpt-x", "-c", "sandbox_mode=\"workspace-write\"", "thread-1", "-"]
        );
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp/proj")));
    }

    #[test]
    fn prompt_travels_on_stdin() {
        assert_eq!(backend().stdin_payload(&req(None)).as_deref(), Some("hi"));
    }

    #[test]
    fn attached_mcp_server_goes_in_as_config_overrides_before_the_thread_id() {
        let mut b = CodexBackend::new("codex", "workspace-write");
        assert!(b.supports_mcp());
        assert!(!args(&b.command(&req(None))).iter().any(|a| a.contains("mcp_servers")));
        b.attach_mcp(&McpAttachment {
            name: "jev".into(),
            command: "/bin/delegate_mcp".into(),
            args: vec!["--dir".into(), "/my repo".into()],
            config_path: "/x.json".into(),
        });
        let fresh = args(&b.command(&req(None)));
        assert!(fresh.contains(&"mcp_servers.jev.command=\"/bin/delegate_mcp\"".to_string()), "{fresh:?}");
        assert!(fresh.contains(&"mcp_servers.jev.args=[\"--dir\",\"/my repo\"]".to_string()), "{fresh:?}");
        let resumed = args(&b.command(&req(Some("t1"))));
        assert_eq!(&resumed[resumed.len() - 2..], ["t1", "-"], "overrides stay before the positionals");
        assert!(resumed.iter().any(|a| a.starts_with("mcp_servers.jev.command")));
    }

    #[test]
    fn command_never_asks_for_less_sandbox_whatever_the_env_says() {
        for junk in ["danger-full-access", "", "READ-ONLY", "workspace-write --dangerously-bypass-approvals-and-sandbox", "bogus"] {
            let b = CodexBackend::new("codex", junk);
            assert_eq!(b.sandbox_arg(), "workspace-write", "{junk:?}");
            for session in [None, Some("t")] {
                let all = args(&b.command(&req(session))).join(" ");
                assert!(!all.contains("danger"), "{all}");
                assert!(!all.contains("bypass"), "{all}");
            }
        }
        assert_eq!(CodexBackend::new("codex", "read-only").sandbox_arg(), "read-only");
    }

    #[test]
    fn provider_is_codex() {
        assert_eq!(backend().provider(), ProviderId::Codex);
    }

    #[test]
    fn classifies_errors() {
        let b = backend();
        assert!(matches!(b.classify_error("You've hit your usage limit"), ErrorKind::Quota { .. }));
        assert_eq!(b.classify_error("unexpected status 401 Unauthorized: Missing bearer or basic authentication"), ErrorKind::Auth);
        assert_eq!(b.classify_error("Missing bearer token"), ErrorKind::Auth);
        assert_eq!(b.classify_error("weird"), ErrorKind::Other);
        let outside_git = "Not inside a trusted directory and --skip-git-repo-check was not specified.";
        assert_eq!(b.classify_error(outside_git), ErrorKind::UnusableHere);
        assert_eq!(b.classify_error("fatal: not a git repository"), ErrorKind::UnusableHere);
    }

    #[test]
    fn allowed_models_are_the_listed_ones() {
        let cache = r#"{"fetched_at":"x","models":[
            {"slug":"gpt-a","visibility":"list"},
            {"slug":"gpt-hidden","visibility":"hide"},
            {"slug":"gpt-b","visibility":"list"},
            {"visibility":"list"}]}"#;
        assert_eq!(allowed_models_from_cache(cache), ["gpt-a", "gpt-b"]);
        assert!(allowed_models_from_cache("nope").is_empty());
        assert!(allowed_models_from_cache("{}").is_empty());
    }
}
