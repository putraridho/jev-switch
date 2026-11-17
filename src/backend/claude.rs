//! Claude Code backend: `claude -p --output-format stream-json --verbose`. The same
//! CLI also serves Anthropic-compatible APIs (DeepSeek) when started with extra env
//! vars, so one struct covers every provider that speaks Claude Code's stream.
//!
//! In a session ([`ClaudeBackend::in_session`], what `jev-code` uses) the CLI runs with
//! `--input-format stream-json --permission-prompt-tool stdio`: instead of denying a tool
//! call that needs approval it sends a `control_request` (`can_use_tool`) on stdout and
//! waits for a `control_response` on stdin, so `jev-code` can ask the person. Recorded
//! lines are in `tests/fixtures/claude/`. The CLI can withdraw a question it asked
//! (`{"type":"control_cancel_request","request_id":...}`, from the protocol schema) when the
//! turn is interrupted or something else settled it; no answer is sent for it then.
//! Headless uses (the gateway, workers, `ask`) and a `jev-code` run nobody can answer
//! (`jev-code -p`, piped input; see [`crate::providers::run_backends`]) keep plain `-p`,
//! where the CLI itself denies what is not allowed.

use std::path::PathBuf;

use serde_json::{json, Value};
use tokio::process::Command;

use super::{
    classify_common, Backend, Decision, ErrorKind, Event, Finish, McpAttachment, PermissionRequest, PlanUsage, ProviderId,
    TurnRequest, Usage,
};
use crate::anthropic::is_limit_message;
use crate::permissions;

/// What the model is told about where it runs. A constant, so the text is byte-identical
/// on every turn and the prompt cache holds.
pub const SESSION_NOTE: &str = "You are running inside jev-code, a terminal router for coding models, not in the Claude Code terminal UI. \
When a tool call needs approval, jev-code asks the user in the terminal by itself: just make the call, and if it is denied, say what you need and why. \
The user manages permission rules with jev-code's /permissions command. \
Never tell the user to use Claude Code interface features such as Shift+Tab, /config or plugin commands: jev-code does not have them.";

pub struct ClaudeBackend {
    pub bin: String,
    pub provider: ProviderId,
    /// Extra environment for the child, e.g. `ANTHROPIC_BASE_URL` for DeepSeek.
    pub env: Vec<(String, String)>,
    pub permission_mode: String,
    pub allowed_tools: Option<String>,
    /// MCP config file passed with `--mcp-config` (the orchestration planner's delegate tool).
    pub mcp_config: Option<PathBuf>,
    /// The attached server's name, to allow its tools without a prompt.
    mcp_name: Option<String>,
    /// Run as a `jev-code` session: tool approvals are asked to the host and the model is
    /// told where it runs.
    interactive: bool,
}

impl ClaudeBackend {
    /// Claude Code with the user's own login. Binary and permissions come from the
    /// environment so every front end (code, gateway workers) behaves the same.
    pub fn claude() -> Self {
        Self::with_profile(ProviderId::Claude, Vec::new())
    }

    /// The same CLI for another provider, pointed at its API through `env`.
    pub fn with_profile(provider: ProviderId, env: Vec<(String, String)>) -> Self {
        Self {
            bin: crate::envfile::lookup("CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            provider,
            env,
            permission_mode: crate::envfile::lookup("CODE_PERMISSION_MODE").unwrap_or_else(|| "acceptEdits".into()),
            allowed_tools: crate::envfile::lookup("CODE_ALLOWED_TOOLS"),
            mcp_config: None,
            mcp_name: None,
            interactive: false,
        }
    }

    /// For a `jev-code` session: the CLI asks the host before a tool that needs approval
    /// (see the module docs) instead of denying it.
    pub fn in_session(mut self) -> Self {
        self.interactive = true;
        self
    }

    /// The `--allowedTools` value: the user's list plus, with an MCP server attached,
    /// that whole server (`mcp__<name>`). Headless `claude -p` cannot ask, so an MCP
    /// tool that is not allowed would simply be denied and the planner could never delegate.
    fn allowed_tools_arg(&self) -> Option<String> {
        let server = self.mcp_name.as_ref().filter(|_| self.mcp_config.is_some()).map(|n| format!("mcp__{n}"));
        match (&self.allowed_tools, server) {
            (Some(a), Some(s)) => Some(format!("{a} {s}")),
            (Some(a), None) => Some(a.clone()),
            (None, s) => s,
        }
    }
}

/// The JSON file `--mcp-config` reads: one stdio server. Pure so the shape is a test.
pub fn mcp_config_json(mcp: &McpAttachment) -> Value {
    json!({ "mcpServers": { mcp.name.as_str(): { "command": mcp.command, "args": mcp.args } } })
}

/// The input field that says most about a tool call: a path, a command, a pattern.
pub fn telling_field(input: &Value) -> Option<&str> {
    ["file_path", "command", "pattern", "path", "url", "description"].iter().find_map(|k| input.get(*k).and_then(Value::as_str))
}

/// One-line summary of a tool call: its name plus the most telling input field.
pub fn tool_summary(name: &str, input: &Value) -> String {
    match telling_field(input) {
        Some(f) => format!("{name} {}", f.chars().take(100).collect::<String>()),
        None => name.to_string(),
    }
}

/// The host's question from a `can_use_tool` control request; other control requests are not ours.
fn permission_request(ev: &Value) -> Option<PermissionRequest> {
    let request = &ev["request"];
    if request["subtype"] != "can_use_tool" {
        return None;
    }
    let tool = request["tool_name"].as_str()?;
    let input = request["input"].clone();
    Some(PermissionRequest {
        id: ev["request_id"].as_str()?.to_string(),
        tool: tool.to_string(),
        summary: permissions::describe(tool, &input),
        blocked_path: request["blocked_path"].as_str().map(String::from),
        input,
    })
}

/// A `control_cancel_request`: the CLI no longer needs the answer to `request_id`. Its shape is
/// `{"type":"control_cancel_request","request_id":"..."}` (Claude Agent SDK control protocol;
/// the schema is in the CLI binary). It may name any control request, not only a permission
/// question; the runner ignores an id that is not waiting.
fn withdrawn_request(ev: &Value) -> Option<Event> {
    Some(Event::PermissionWithdrawn { id: ev["request_id"].as_str()?.to_string() })
}

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Everything sent in one call: fresh input plus cache reads plus cache writes.
fn total_input(v: &Value) -> u64 {
    count(v, "input_tokens") + count(v, "cache_read_input_tokens") + count(v, "cache_creation_input_tokens")
}

/// `usage` sums every API call in the turn; the context size is the last call's
/// input, so it comes from the final `iterations` entry.
fn usage_of(result: &Value) -> Usage {
    let u = &result["usage"];
    let last = u["iterations"].as_array().and_then(|a| a.last()).unwrap_or(u);
    Usage {
        input_tokens: total_input(u),
        cached_input_tokens: count(u, "cache_read_input_tokens"),
        output_tokens: count(u, "output_tokens"),
        context_tokens: Some(total_input(last)),
    }
}

fn assistant_events(ev: &Value) -> Vec<Event> {
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter_map(|b| match b["type"].as_str() {
            Some("tool_use") => Some(Event::Tool(tool_summary(b["name"].as_str().unwrap_or("?"), &b["input"]))),
            Some("text") => {
                let t = b["text"].as_str().unwrap_or("").trim();
                // The limit notice is reported as an error by the runner, not shown as an answer.
                (!t.is_empty() && !is_limit_message(t)).then(|| Event::Text(t.to_string()))
            }
            _ => None,
        })
        .collect()
}

fn result_event(ev: &Value) -> Event {
    let text = ev["result"].as_str().unwrap_or("").to_string();
    if ev["is_error"].as_bool() == Some(true) {
        let message = if text.trim().is_empty() { "claude reported an error".to_string() } else { text };
        return Event::Failed { message };
    }
    Event::Finished(Finish { text, session_id: ev["session_id"].as_str().map(String::from), usage: usage_of(ev) })
}

/// The busiest plan window (highest utilization) from a `rate_limit_event`.
fn plan_of(ev: &Value) -> Option<PlanUsage> {
    let windows = ev["rate_limit_info"]["unifiedWindows"].as_object()?;
    let (name, w) = windows
        .iter()
        .filter_map(|(k, w)| Some((k, w, w["utilization"].as_f64()?)))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(k, w, _)| (k, w))?;
    Some(PlanUsage {
        window: name.clone(),
        utilization: w["utilization"].as_f64()?,
        resets_at: w["resetsAt"].as_u64(),
    })
}

impl Backend for ClaudeBackend {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-p", "--model", &req.model, "--output-format", "stream-json", "--verbose"])
            .current_dir(&req.dir)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        cmd.args(["--permission-mode", &self.permission_mode]);
        if self.interactive {
            cmd.args(["--input-format", "stream-json", "--permission-prompt-tool", "stdio", "--append-system-prompt", SESSION_NOTE]);
        }
        if let Some(config) = &self.mcp_config {
            cmd.arg("--mcp-config").arg(config);
        }
        if let Some(allowed) = self.allowed_tools_arg() {
            cmd.args(["--allowedTools", &allowed]);
        }
        cmd
    }

    /// Stream-json input is one JSON message per line, not the bare prompt.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        if !self.interactive {
            return Some(req.prompt.clone());
        }
        Some(format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": req.prompt } })))
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        let answer = match decision {
            Decision::Allow | Decision::AllowAlways => json!({ "behavior": "allow", "updatedInput": req.input }),
            Decision::Deny { reason } => json!({ "behavior": "deny", "message": reason }),
        };
        let line = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": req.id, "response": answer } });
        Some(line.to_string())
    }

    /// DeepSeek runs through the same CLI, so it can have the tool attached too.
    fn supports_mcp(&self) -> bool {
        true
    }

    fn attach_mcp(&mut self, mcp: &McpAttachment) {
        self.mcp_config = Some(mcp.config_path.clone());
        self.mcp_name = Some(mcp.name.clone());
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("assistant") => assistant_events(&ev),
            Some("system") if ev["subtype"] == "permission_denied" => vec![Event::Hint(format!(
                "\u{2717} denied {} \u{2014} allow it via CODE_ALLOWED_TOOLS or your Claude Code settings",
                ev["tool_name"].as_str().unwrap_or("tool")
            ))],
            Some("result") => vec![result_event(&ev)],
            Some("control_request") => permission_request(&ev).map(Event::PermissionRequest).into_iter().collect(),
            Some("control_cancel_request") => withdrawn_request(&ev).into_iter().collect(),
            Some("rate_limit_event") => plan_of(&ev).map(Event::Plan).into_iter().collect(),
            _ => vec![],
        }
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        classify_common(message).unwrap_or(ErrorKind::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TURN1: &str = include_str!("../../tests/fixtures/claude/turn1.jsonl");

    fn parse_all(backend: &ClaudeBackend) -> Vec<Event> {
        TURN1.lines().flat_map(|l| backend.parse_line(l)).collect()
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn backend() -> ClaudeBackend {
        ClaudeBackend {
            bin: "claude".into(),
            provider: ProviderId::Claude,
            env: vec![],
            permission_mode: "acceptEdits".into(),
            allowed_tools: None,
            mcp_config: None,
            mcp_name: None,
            interactive: false,
        }
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "sonnet-x".into(),
            prompt: "hi".into(),
            session_id: session.map(String::from),
        }
    }

    #[test]
    fn recorded_turn_parses_to_tool_plan_text_finish() {
        let events = parse_all(&backend());
        assert_eq!(events.len(), 4, "{events:?}");
        assert!(matches!(&events[0], Event::Tool(s) if s.starts_with("Read ") && s.ends_with("/spike/a.txt")));
        assert!(matches!(&events[1], Event::Plan(p)
            if p.window == "seven_day" && (p.utilization - 0.29).abs() < 1e-9 && p.resets_at == Some(1795114800)));
        assert_eq!(events[2], Event::Text("hello".into()));
        let Event::Finished(f) = &events[3] else { panic!("{:?}", events[3]) };
        assert_eq!(f.text, "hello");
        assert_eq!(f.session_id.as_deref(), Some("03ede922-7bce-402e-b690-5271724d91f6"));
        // Turn total is 4 + 35536 + 14188; the context is the last iteration only: 2 + 24812 + 100.
        assert_eq!(
            f.usage,
            Usage { input_tokens: 49728, cached_input_tokens: 35536, output_tokens: 85, context_tokens: Some(24914) }
        );
    }

    #[test]
    fn context_falls_back_to_usage_without_iterations() {
        let line = json!({"type":"result","result":"ok","usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}});
        let Event::Finished(f) = &backend().parse_line(&line.to_string())[0] else { panic!() };
        assert_eq!(f.usage.context_tokens, Some(6));
    }

    #[test]
    fn error_result_is_failed() {
        let line = json!({"type":"result","is_error":true,"result":"boom"}).to_string();
        assert_eq!(backend().parse_line(&line), vec![Event::Failed { message: "boom".into() }]);
    }

    #[test]
    fn permission_denied_gives_a_hint_that_is_not_answer_text() {
        let line = json!({"type":"system","subtype":"permission_denied","tool_name":"Bash"}).to_string();
        let evs = backend().parse_line(&line);
        assert!(matches!(&evs[..], [Event::Hint(t)] if t.contains("denied Bash") && t.contains("CODE_ALLOWED_TOOLS")));
    }

    #[test]
    fn limit_notice_text_is_not_shown() {
        let line = json!({"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your session limit"}]}});
        assert!(backend().parse_line(&line.to_string()).is_empty());
    }

    #[test]
    fn junk_and_init_lines_are_ignored() {
        assert!(backend().parse_line("not json").is_empty());
        assert!(backend().parse_line("").is_empty());
        assert!(backend().parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
        assert!(backend().parse_line(r#"{"type":"rate_limit_event","rate_limit_info":{}}"#).is_empty());
    }

    #[test]
    fn tool_summary_picks_the_telling_field() {
        assert_eq!(tool_summary("Bash", &json!({"command":"cargo test"})), "Bash cargo test");
        assert_eq!(tool_summary("Task", &json!({})), "Task");
        let long = "x".repeat(300);
        assert_eq!(tool_summary("Read", &json!({"file_path": long})).chars().count(), 105);
    }

    #[test]
    fn command_matches_the_old_inline_invocation() {
        assert_eq!(
            args(&backend().command(&req(None))),
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--permission-mode", "acceptEdits"]
        );
        let b = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
        assert_eq!(
            args(&b.command(&req(Some("abc")))),
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--resume", "abc",
             "--permission-mode", "acceptEdits", "--allowedTools", "Bash(cargo *)"]
        );
    }

    fn attachment() -> McpAttachment {
        McpAttachment {
            name: "jev".into(),
            command: "/bin/delegate_mcp".into(),
            args: vec!["--dir".into(), "/repo".into()],
            config_path: "/state/mcp.json".into(),
        }
    }

    #[test]
    fn attached_mcp_config_is_passed_and_its_tools_allowed() {
        let mut b = backend();
        assert!(b.supports_mcp());
        b.attach_mcp(&attachment());
        let a = args(&b.command(&req(None)));
        let i = a.iter().position(|x| x == "--mcp-config").expect("flag");
        assert_eq!(a[i + 1], "/state/mcp.json");
        let j = a.iter().position(|x| x == "--allowedTools").expect("server allowed");
        assert_eq!(a[j + 1], "mcp__jev");
        let mut b = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
        b.attach_mcp(&attachment());
        let a = args(&b.command(&req(None)));
        assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *) mcp__jev");
        // Without an attachment the command is the one it always was.
        assert!(!args(&backend().command(&req(None))).iter().any(|x| x.contains("mcp")));
    }

    #[test]
    fn mcp_config_file_runs_the_server_command() {
        let v = mcp_config_json(&attachment());
        assert_eq!(v["mcpServers"]["jev"]["command"], "/bin/delegate_mcp");
        assert_eq!(v["mcpServers"]["jev"]["args"], json!(["--dir", "/repo"]));
    }

    #[test]
    fn profile_env_reaches_the_command() {
        let b = ClaudeBackend { env: vec![("ANTHROPIC_BASE_URL".into(), "https://x".into())], provider: ProviderId::DeepSeek, ..backend() };
        let cmd = b.command(&req(None));
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(envs.iter().any(|(k, v)| *k == "ANTHROPIC_BASE_URL" && v.is_some_and(|v| v == "https://x")));
        assert_eq!(b.provider(), ProviderId::DeepSeek);
    }

    #[test]
    fn classifies_common_errors() {
        assert!(matches!(backend().classify_error("You've hit your limit"), ErrorKind::Quota { .. }));
        assert_eq!(backend().classify_error("weird"), ErrorKind::Other);
    }

    // ------------------------------------------------------------- host approvals

    const ALLOW_TURN: &str = include_str!("../../tests/fixtures/claude/permission_allow.jsonl");
    const DENY_TURN: &str = include_str!("../../tests/fixtures/claude/permission_deny.jsonl");
    const ALLOW_LINE: &str = include_str!("../../tests/fixtures/claude/control_response_allow.jsonl");
    const DENY_LINE: &str = include_str!("../../tests/fixtures/claude/control_response_deny.jsonl");
    const USER_LINE: &str = include_str!("../../tests/fixtures/claude/user_message.jsonl");

    fn session() -> ClaudeBackend {
        backend().in_session()
    }

    fn request_from(turn: &str) -> PermissionRequest {
        let events: Vec<Event> = turn.lines().flat_map(|l| backend().parse_line(l)).collect();
        events
            .into_iter()
            .find_map(|e| match e {
                Event::PermissionRequest(r) => Some(r),
                _ => None,
            })
            .expect("the recorded turn holds a control request")
    }

    fn json_of(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn the_recorded_control_request_becomes_a_permission_request() {
        let r = request_from(ALLOW_TURN);
        assert_eq!(r.id, "2f318992-1d40-4a9f-afab-303ef045d9d6");
        assert_eq!(r.tool, "Bash");
        assert_eq!(r.summary, "touch /tmp/spike/outside.txt");
        assert_eq!(r.blocked_path.as_deref(), Some("/tmp/spike/outside.txt"));
        assert_eq!(r.input["command"], "touch /tmp/spike/outside.txt");
        // The whole recorded turn: the request, and the denial event that old headless runs got is absent.
        let events: Vec<Event> = ALLOW_TURN.lines().flat_map(|l| backend().parse_line(l)).collect();
        assert_eq!(events.iter().filter(|e| matches!(e, Event::PermissionRequest(_))).count(), 1);
        assert!(matches!(events.last(), Some(Event::Finished(_))), "{events:?}");
    }

    #[test]
    fn the_request_summary_and_debug_leave_secrets_out_of_logs() {
        let line = json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"curl -H 'Authorization: Bearer SECRET'"}}}).to_string();
        let Event::PermissionRequest(r) = &backend().parse_line(&line)[0] else { panic!() };
        assert!(r.summary.contains("SECRET"), "the person sees what they are approving");
        let debug = format!("{r:?}");
        assert!(!debug.contains("SECRET") && debug.contains("Bash") && debug.contains("r1"), "{debug}");
    }

    #[test]
    fn writes_are_summarised_by_path_and_size_not_contents() {
        let line = json!({"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/p/.claude/settings.local.json","content":"{\"permissions\": {}}"}}}).to_string();
        let Event::PermissionRequest(r) = &backend().parse_line(&line)[0] else { panic!() };
        assert_eq!(r.summary, "/p/.claude/settings.local.json (19 bytes)");
    }

    #[test]
    fn other_control_requests_and_malformed_ones_are_not_questions() {
        assert!(backend().parse_line(r#"{"type":"control_request","request_id":"x","request":{"subtype":"hook_callback"}}"#).is_empty());
        assert!(backend().parse_line(r#"{"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#).is_empty());
        assert!(backend().parse_line(r#"{"type":"control_cancel_request"}"#).is_empty(), "no id, nothing to withdraw");
    }

    #[test]
    fn a_cancel_request_withdraws_its_request_id() {
        // The shape the SDK control protocol documents; the CLI does not send it in the recorded turns.
        let line = json!({"type":"control_cancel_request","request_id":"2f318992-1d40-4a9f-afab-303ef045d9d6"}).to_string();
        assert_eq!(
            backend().parse_line(&line),
            vec![Event::PermissionWithdrawn { id: "2f318992-1d40-4a9f-afab-303ef045d9d6".into() }]
        );
    }

    #[test]
    fn allow_answers_with_the_original_input_as_the_recorded_line_does() {
        let r = request_from(ALLOW_TURN);
        for decision in [Decision::Allow, Decision::AllowAlways] {
            let line = session().permission_response(&r, &decision).unwrap();
            assert!(!line.contains('\n'), "one line");
            assert_eq!(json_of(&line), json_of(ALLOW_LINE));
        }
    }

    #[test]
    fn deny_answers_with_the_reason_as_the_recorded_line_does() {
        let r = request_from(DENY_TURN);
        let line = session().permission_response(&r, &Decision::Deny { reason: "User denied via host".into() }).unwrap();
        assert_eq!(json_of(&line), json_of(DENY_LINE));
    }

    #[test]
    fn a_session_runs_with_stream_json_input_a_host_prompt_tool_and_the_note() {
        let a = args(&session().command(&req(None)));
        assert_eq!(
            a,
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--permission-mode", "acceptEdits",
             "--input-format", "stream-json", "--permission-prompt-tool", "stdio", "--append-system-prompt", SESSION_NOTE]
        );
        assert!(session().interactive());
        // Headless use is what it always was.
        assert!(!backend().interactive());
        assert!(!args(&backend().command(&req(None))).iter().any(|x| x.contains("input-format") || x.contains("system-prompt")));
    }

    #[test]
    fn the_prompt_travels_as_a_stream_json_user_message() {
        let mut request = req(None);
        request.prompt = "Use the Bash tool to run exactly: touch /tmp/spike/outside.txt   Then reply with just the word done.".into();
        let payload = session().stdin_payload(&request).unwrap();
        assert!(payload.ends_with('\n') && payload.matches('\n').count() == 1);
        assert_eq!(json_of(&payload), json_of(USER_LINE));
        // A multi-line prompt stays on one line.
        request.prompt = "a\nb \"c\"".into();
        let payload = session().stdin_payload(&request).unwrap();
        assert_eq!(payload.matches('\n').count(), 1);
        assert_eq!(json_of(&payload)["message"]["content"], "a\nb \"c\"");
        // Headless: the bare prompt, as before.
        assert_eq!(backend().stdin_payload(&request).as_deref(), Some("a\nb \"c\""));
    }

    #[test]
    fn the_note_is_short_stable_and_steers_away_from_claude_code_ui() {
        assert!(SESSION_NOTE.len() < 700, "{}", SESSION_NOTE.len());
        assert!(SESSION_NOTE.contains("jev-code") && SESSION_NOTE.contains("/permissions") && SESSION_NOTE.contains("Shift+Tab"));
        assert!(!SESSION_NOTE.contains('\n'), "no stray line breaks from the source layout");
        let a = args(&session().command(&req(Some("abc"))));
        let b = args(&session().command(&req(None)));
        let note = |v: &[String]| v[v.iter().position(|x| x == "--append-system-prompt").unwrap() + 1].clone();
        assert_eq!(note(&a), note(&b), "byte-identical every turn");
    }
}
