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
The user manages permission rules with jev-code's /permissions command and the permission mode (ask, edit, plan, auto) with /mode or Shift+Tab. \
Never tell the user to use Claude Code interface features such as /config or plugin commands: jev-code does not have them. \
Use the Read, Grep, Glob and Edit tools to look at and change files, not sed, python or heredocs, and keep shell commands plain: scripts and heredocs are asked about every time. \
Ask the user questions in your reply, not with a tool. \
Wait for commands to finish before you answer: you cannot speak again until the user writes, so never promise to report back later.";

/// Turns off Claude Code's background shells and monitors (see [`ClaudeBackend::command`]).
const NO_BACKGROUND_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS";

/// The environment variable that sets Claude Code's thinking budget in tokens.
const THINKING_ENV: &str = "MAX_THINKING_TOKENS";

pub struct ClaudeBackend {
    pub bin: String,
    pub provider: ProviderId,
    /// Extra environment for the child, e.g. `ANTHROPIC_BASE_URL` for DeepSeek.
    pub env: Vec<(String, String)>,
    pub allowed_tools: Option<String>,
    /// MCP config file passed with `--mcp-config` (the orchestration planner's delegate tool).
    pub mcp_config: Option<PathBuf>,
    /// The attached server's name, to allow its tools without a prompt.
    mcp_name: Option<String>,
    /// Run as a `jev-code` session: tool approvals are asked to the host and the model is
    /// told where it runs.
    interactive: bool,
    /// Ask for partial assistant text (`--include-partial-messages`), for the live screen.
    streaming: bool,
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
            allowed_tools: crate::envfile::lookup("CODE_ALLOWED_TOOLS"),
            mcp_config: None,
            mcp_name: None,
            interactive: false,
            streaming: false,
        }
    }

    /// For a `jev-code` session: the CLI asks the host before a tool that needs approval
    /// (see the module docs) instead of denying it.
    pub fn in_session(mut self) -> Self {
        self.interactive = true;
        self
    }

    /// The user's tools plus the attached server when the mode permits auto-approval.
    fn allowed_tools_arg(&self, mode: crate::mode::Mode) -> Option<String> {
        let server = self.mcp_name.as_ref().filter(|_| self.mcp_config.is_some() && matches!(mode, crate::mode::Mode::Edit | crate::mode::Mode::Auto)).map(|n| format!("mcp__{n}"));
        match (&self.allowed_tools, server) {
            (Some(a), Some(s)) => Some(format!("{a} {s}")),
            (Some(a), None) => Some(a.clone()),
            (None, s) => s,
        }
    }
}

/// Claude settings sources permitted for this project.
fn setting_sources(local_trusted: bool, project_trusted: bool) -> &'static str {
    match (local_trusted, project_trusted) {
        (false, false) => "user",
        (true, false) => "user,local",
        (false, true) => "user,project",
        (true, true) => "user,project,local",
    }
}

/// The JSON file `--mcp-config` reads: one stdio server. Pure so the shape is a test.
pub fn mcp_config_json(mcp: &McpAttachment) -> Value {
    json!({ "mcpServers": { mcp.name.as_str(): { "command": mcp.command, "args": mcp.args } } })
}

/// Write the JSON file Claude Code's `--mcp-config` reads. Harmless for other CLIs.
pub fn write_mcp_config(mcp: &McpAttachment) -> std::io::Result<()> {
    let json = crate::backend::claude::mcp_config_json(mcp);
    std::fs::write(&mcp.config_path, json.to_string())
}

/// The input field that says most about a tool call: a path, a command, a pattern.
pub fn telling_field(input: &Value) -> Option<&str> {
    ["file_path", "command", "pattern", "path", "url", "description", "brief", "worker_id"].iter().find_map(|k| input.get(*k).and_then(Value::as_str))
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
    let nested = !ev["parent_tool_use_id"].is_null();
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    let mut out = Vec::new();
    for b in blocks {
        match b["type"].as_str() {
            Some("tool_use") => {
                let name = b["name"].as_str().unwrap_or("?");
                out.push(Event::Tool(tool_summary(name, &b["input"])));
                if !nested {
                    out.extend(edit_event(name, &b["input"]));
                }
            }
            Some("text") if !nested => {
                let t = b["text"].as_str().unwrap_or("").trim();
                // The limit notice is reported as an error by the runner, not shown as an answer.
                if !t.is_empty() && !is_limit_message(t) {
                    out.push(Event::Text(t.to_string()));
                }
            }
            Some("thinking") if !nested => {
                let t = b["thinking"].as_str().unwrap_or("").trim();
                if !t.is_empty() {
                    out.push(Event::Thinking(t.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// A piece of the answer as Claude writes it (`--include-partial-messages`). Only top-level
/// text deltas: a subagent's text, thinking and tool input stay out. The whole message still
/// arrives as `assistant` and is what the turn commits.
fn stream_delta(ev: &Value) -> Vec<Event> {
    if !ev["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    let inner = &ev["event"];
    if inner["type"] != "content_block_delta" || inner["delta"]["type"] != "text_delta" {
        return Vec::new();
    }
    let text = inner["delta"]["text"].as_str().unwrap_or("");
    (!text.is_empty()).then(|| Event::TextDelta(text.to_string())).into_iter().collect()
}

fn lines_of(v: &Value) -> Vec<String> {    v.as_str().map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
}

/// What an Edit, MultiEdit or Write call changes, from its input.
fn edit_event(tool: &str, input: &Value) -> Option<Event> {
    let path = input["file_path"].as_str()?.to_string();
    match tool {
        "Edit" => Some(Event::Edit { path, removed: lines_of(&input["old_string"]), added: lines_of(&input["new_string"]) }),
        "MultiEdit" => {
            let edits = input["edits"].as_array()?;
            let removed = edits.iter().flat_map(|e| lines_of(&e["old_string"])).collect();
            let added = edits.iter().flat_map(|e| lines_of(&e["new_string"])).collect();
            Some(Event::Edit { path, removed, added })
        }
        "Write" => Some(Event::Edit { path, removed: Vec::new(), added: lines_of(&input["content"]) }),
        _ => None,
    }
}

/// The text of a `tool_result` block: a string, or a list of text parts.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Tool results of the main conversation (a sub-agent's are its own business).
fn user_events(ev: &Value) -> Vec<Event> {
    if !ev["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    // `--replay-user-messages`: a message from stdin, echoed when the model has it.
    if ev["isReplay"].as_bool() == Some(true) {
        let content = &ev["message"]["content"];
        let text = match content.as_str() {
            Some(text) => text.to_string(),
            None => content.as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        };
        return vec![Event::Steered(text)];
    }
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter(|b| b["type"] == "tool_result")
        .map(|b| Event::ToolResult { ok: b["is_error"].as_bool() != Some(true), text: result_text(&b["content"]) })
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

// ------------------------------------------------------------- stream-json input

/// One line of stream-json input: the user's message.
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InputLine<'a> {
    User { message: InputMessage<'a> },
}

#[derive(serde::Serialize)]
struct InputMessage<'a> {
    role: Role,
    content: InputContent<'a>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
}

/// A plain prompt is a string; one with images is a list of blocks.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum InputContent<'a> {
    Text(&'a str),
    Blocks(Vec<InputBlock<'a>>),
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InputBlock<'a> {
    Image { source: ImageSource },
    Text { text: &'a str },
}

#[derive(serde::Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    encoding: Encoding,
    media_type: MediaType,
    data: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Encoding {
    Base64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum MediaType {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
}

impl MediaType {
    /// The type the file's first bytes say, whatever its name: the API refuses an image
    /// whose declared type is wrong (a JPEG saved as `.png`).
    fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Self::Jpeg)
        } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }
}

/// Whether a request's images go as image blocks: Claude takes them; DeepSeek's API does
/// not, so it gets their paths in the prompt instead.
fn sends_image_blocks(provider: ProviderId, req: &TurnRequest) -> bool {
    !req.images.is_empty() && provider != ProviderId::DeepSeek
}

/// The largest image sent; bigger ones are left out with a note in the prompt. The API's
/// 5 MB limit counts the base64 text, which is 4/3 of the file.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024 * 3 / 4;

/// One image block, or the note that says why it was left out.
fn image_block(path: &std::path::Path) -> Result<ImageSource, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Could not read image {}: {e}", path.display()))?.len();
    if size > MAX_IMAGE_BYTES {
        return Err(format!("Left out an image over 5 MB: {}", path.display()));
    }
    let data = std::fs::read(path).map_err(|e| format!("Could not read image {}: {e}", path.display()))?;
    let media_type = MediaType::sniff(&data).ok_or_else(|| format!("Left out a file that is not a PNG, JPEG, GIF or WebP image: {}", path.display()))?;
    Ok(ImageSource { encoding: Encoding::Base64, media_type, data: crate::base64::encode(&data) })
}

/// The image blocks of a request, and the prompt with a note for each image left out.
fn image_content(req: &TurnRequest) -> (Vec<InputBlock<'static>>, String) {
    let mut blocks = Vec::new();
    let mut prompt = req.prompt.clone();
    for path in &req.images {
        let path = if path.is_absolute() { path.clone() } else { req.dir.join(path) };
        match image_block(&path) {
            Ok(source) => blocks.push(InputBlock::Image { source }),
            Err(note) => prompt.push_str(&format!("\n({note})")),
        }
    }
    (blocks, prompt)
}

impl Backend for ClaudeBackend {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-p", "--model", &req.model, "--output-format", "stream-json", "--verbose"])
            .current_dir(&req.dir)
            // A turn is one process: background shells and monitors would outlive its result,
            // get killed when the process is reaped, and start turns of their own after stdin
            // is closed, where every tool approval fails with "Stream closed".
            .env(NO_BACKGROUND_ENV, "1")
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if self.streaming {
            cmd.arg("--include-partial-messages");
        }
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        let places = crate::ext::Places::from_env(&req.dir);
        let project_trusted = places.user.as_deref().is_some_and(|user| crate::ext::hooks::claude_settings_trusted(user, &req.dir));
        let sources = setting_sources(!permissions::local_settings_tracked(&req.dir), project_trusted);
        cmd.args(["--setting-sources", sources, "--permission-mode", req.mode.claude_flag()]);
        if self.interactive || sends_image_blocks(self.provider, req) {
            cmd.args(["--input-format", "stream-json"]);
        }
        if self.interactive {
            // The CLI echoes each message sent on stdin once the model has it: how a message
            // typed during the turn is known to have been taken (see `steer_line`).
            cmd.arg("--replay-user-messages");
            // AskUserQuestion needs Claude Code's own screen: here it would reach the person as
            // raw JSON to approve, with no way to answer it.
            cmd.args(["--permission-prompt-tool", "stdio", "--disallowedTools", "AskUserQuestion", "--append-system-prompt", SESSION_NOTE]);
        }
        if req.mode != crate::mode::Mode::Plan {
            if let Some(config) = &self.mcp_config {
                cmd.arg("--mcp-config").arg(config);
            }
        }
        if let Some(allowed) = self.allowed_tools_arg(req.mode) {
            cmd.args(["--allowedTools", &allowed]);
        }
        cmd
    }

    /// Stream-json input is one JSON message per line, not the bare prompt.
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        let blocks_wanted = sends_image_blocks(self.provider, req);
        if !self.interactive && !blocks_wanted {
            return Some(super::prompt_with_images(req));
        }
        let (mut blocks, prompt) = if blocks_wanted { image_content(req) } else { (Vec::new(), super::prompt_with_images(req)) };
        let content = if !blocks_wanted {
            InputContent::Text(&prompt)
        } else {
            blocks.push(InputBlock::Text { text: &prompt });
            InputContent::Blocks(blocks)
        };
        let line = InputLine::User { message: InputMessage { role: Role::User, content } };
        serde_json::to_string(&line).ok().map(|s| format!("{s}\n"))
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn steerable(&self) -> bool {
        self.interactive
    }

    fn steer_line(&self, text: &str) -> Option<String> {
        if !self.interactive {
            return None;
        }
        let line = InputLine::User { message: InputMessage { role: Role::User, content: InputContent::Text(text) } };
        serde_json::to_string(&line).ok()
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

    /// Claude Code reads its thinking budget from `MAX_THINKING_TOKENS`.
    fn set_effort(&mut self, effort: crate::effort::Effort) {
        self.env.retain(|(k, _)| k != THINKING_ENV);
        self.env.push((THINKING_ENV.to_string(), effort.thinking_tokens().to_string()));
    }

    fn set_streaming(&mut self, on: bool) {
        self.streaming = on;
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("assistant") => assistant_events(&ev),
            Some("stream_event") => stream_delta(&ev),
            Some("user") => user_events(&ev),
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

    const TURN1: &str = fixture!("claude/turn1.jsonl");

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
            allowed_tools: None,
            mcp_config: None,
            mcp_name: None,
            interactive: false,
            streaming: false,
        }
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "sonnet-x".into(),
            prompt: "hi".into(),
            images: vec![],
            session_id: session.map(String::from),
            mode: Default::default(),
        }
    }

    #[test]
    fn image_input_and_base64() {
        let backend = backend().in_session();
        assert_eq!(backend.stdin_payload(&req(None)).unwrap(), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        let dir = std::env::temp_dir().join(format!("jev-claude-image-{}", std::process::id()));
        // A JPEG saved as .png goes as what it really is.
        std::fs::write(&dir, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        let path = dir.with_extension("png");
        std::fs::rename(&dir, &path).unwrap();
        let mut request = req(None);
        request.images.push(path.clone());
        let payload = backend.stdin_payload(&request).unwrap();
        assert!(payload.contains("\"type\":\"image\"") && payload.contains("\"media_type\":\"image/jpeg\""), "{payload}");
        assert!(payload.contains("\"data\":\"/9j/4A==\""), "{payload}");
        // Text with an image name is left out with a note, never sent as an image.
        let fake = std::env::temp_dir().join(format!("jev-claude-fake-{}.png", std::process::id()));
        std::fs::write(&fake, b"not an image").unwrap();
        let mut faked = req(None);
        faked.images.push(fake.clone());
        let note = backend.stdin_payload(&faked).unwrap();
        assert!(!note.contains("\"type\":\"image\"") && note.contains("not a PNG, JPEG, GIF or WebP"), "{note}");
        std::fs::remove_file(fake).unwrap();
        assert_eq!(MediaType::sniff(b"\x89PNG\r\n\x1a\nrest"), Some(MediaType::Png));
        assert_eq!(MediaType::sniff(b"GIF89a"), Some(MediaType::Gif));
        assert_eq!(MediaType::sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(MediaType::Webp));
        assert_eq!(MediaType::sniff(b"RIFF"), None);
        // DeepSeek's API takes no images: it gets their paths as text.
        let deepseek = ClaudeBackend::with_profile(ProviderId::DeepSeek, vec![]).in_session();
        let text = deepseek.stdin_payload(&request).unwrap();
        assert!(!text.contains("\"type\":\"image\"") && text.contains(&*path.to_string_lossy()), "{text}");
        let plain = ClaudeBackend::with_profile(ProviderId::DeepSeek, vec![]);
        let args: Vec<String> = plain.command(&request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(!args.contains(&"stream-json".to_string()) || args.windows(2).all(|w| w != ["--input-format", "stream-json"]), "{args:?}");
        assert!(plain.stdin_payload(&request).unwrap().contains("Attached images"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn plan_mode_is_read_only_even_when_nobody_can_be_asked() {
        let headless = backend();
        let mut r = req(None);
        r.mode = crate::mode::Mode::Plan;
        let args: Vec<String> = headless.command(&r).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]), "{args:?}");
    }

    #[test]
    fn recorded_turn_parses_to_tool_plan_text_finish() {
        let all = parse_all(&backend());
        assert!(all.contains(&Event::ToolResult { ok: true, text: "1\thello\n2\t\n3\tworld\n4\t".into() }), "{all:?}");
        let events: Vec<Event> = all.into_iter().filter(|e| !matches!(e, Event::ToolResult { .. })).collect();
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
    fn subagent_text_is_never_answer_text() {
        let line = json!({"type":"assistant","parent_tool_use_id":"agent-1","message":{"content":[
            {"type":"text","text":"subagent report"}, {"type":"tool_use","name":"Read","input":{"file_path":"a"}}
        ]}});
        assert_eq!(backend().parse_line(&line.to_string()), vec![Event::Tool("Read a".into())]);
        let result = json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","content":"subagent report"}]}});
        let events = backend().parse_line(&result.to_string());
        assert!(!events.iter().any(|e| matches!(e, Event::Text(_))), "a sub-agent's report is a tool result, not the answer: {events:?}");
    }

    #[test]
    fn partial_text_is_the_live_preview_and_never_a_subagent_or_thinking() {
        let line = json!({"type":"stream_event","parent_tool_use_id":null,
            "event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}});
        assert_eq!(backend().parse_line(&line.to_string()), vec![Event::TextDelta("Hel".into())]);
        for event in [
            json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}),
            json!({"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{}"}}),
            json!({"type":"message_start"}),
        ] {
            let line = json!({"type":"stream_event","parent_tool_use_id":null,"event":event});
            assert_eq!(backend().parse_line(&line.to_string()), vec![], "not answer text: {line}");
        }
        let nested = json!({"type":"stream_event","parent_tool_use_id":"agent-1",
            "event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sub"}}});
        assert_eq!(backend().parse_line(&nested.to_string()), vec![], "a subagent's text never joins the answer");
    }

    #[test]
    fn partial_messages_are_requested_only_when_streaming() {
        let mut b = backend();
        let plain = args(&b.command(&req(None)));
        assert!(!plain.iter().any(|a| a == "--include-partial-messages"), "{plain:?}");
        b.set_streaming(true);
        let streamed = args(&b.command(&req(None)));
        assert!(streamed.iter().any(|a| a == "--include-partial-messages"), "{streamed:?}");
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
    fn tool_results_and_edits_of_the_main_conversation_are_reported() {
        let b = backend();
        let ok = json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"line one\nline two"}]}});
        assert_eq!(b.parse_line(&ok.to_string()), vec![Event::ToolResult { ok: true, text: "line one\nline two".into() }]);
        let failed = json!({"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"no such file"}]}]}});
        assert_eq!(b.parse_line(&failed.to_string()), vec![Event::ToolResult { ok: false, text: "no such file".into() }]);
        let nested = json!({"type":"user","parent_tool_use_id":"agent","message":{"content":[{"type":"tool_result","content":"x"}]}});
        assert!(b.parse_line(&nested.to_string()).is_empty(), "a sub-agent's results stay inside it");
        let edit = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"src/a.rs","old_string":"let a = 1;","new_string":"let a = 2;\nlet b = 3;"}}]}});
        assert_eq!(
            b.parse_line(&edit.to_string()),
            vec![Event::Tool("Edit src/a.rs".into()), Event::Edit { path: "src/a.rs".into(), removed: vec!["let a = 1;".into()], added: vec!["let a = 2;".into(), "let b = 3;".into()] }]
        );
        let write = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"n.txt","content":"hi\n"}}]}});
        assert!(matches!(&b.parse_line(&write.to_string())[1], Event::Edit { removed, added, .. } if removed.is_empty() && added == &vec!["hi".to_string()]));
    }

    #[test]
    fn thinking_is_reported_apart_from_the_answer() {
        let line = json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"The empty case returns early."},{"type":"text","text":"Fixed."}]}});
        assert_eq!(backend().parse_line(&line.to_string()), vec![Event::Thinking("The empty case returns early.".into()), Event::Text("Fixed.".into())]);
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
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--setting-sources", "user,local", "--permission-mode", "acceptEdits"]
        );
        let b = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
        assert_eq!(
            args(&b.command(&req(Some("abc")))),
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--resume", "abc",
             "--setting-sources", "user,local", "--permission-mode", "acceptEdits", "--allowedTools", "Bash(cargo *)"]
        );
    }

    #[test]
    fn every_run_follows_the_requested_mode() {
        use crate::mode::Mode;
        let session = backend().in_session();
        for (mode, flag) in [(Mode::Ask, "default"), (Mode::Edit, "acceptEdits"), (Mode::Plan, "plan"), (Mode::Auto, "acceptEdits")] {
            for b in [&session, &backend()] {
                let a = args(&b.command(&TurnRequest { mode, ..req(None) }));
                let at = a.iter().position(|x| x == "--permission-mode").unwrap();
                assert_eq!(a[at + 1], flag, "{mode}");
            }
        }
    }

    #[test]
    fn setting_sources_respect_project_and_local_trust() {
        assert_eq!(setting_sources(false, false), "user");
        assert_eq!(setting_sources(true, false), "user,local");
        assert_eq!(setting_sources(false, true), "user,project");
        assert_eq!(setting_sources(true, true), "user,project,local");
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
        for mode in [crate::mode::Mode::Ask, crate::mode::Mode::Plan] {
            let a = args(&b.command(&TurnRequest { mode, ..req(None) }));
            assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *)");
            assert_eq!(a.iter().any(|x| x == "--mcp-config"), mode == crate::mode::Mode::Ask);
        }
        let a = args(&b.command(&TurnRequest { mode: crate::mode::Mode::Auto, ..req(None) }));
        assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *) mcp__jev");
        // Without an attachment the command is the one it always was.
        assert!(!args(&backend().command(&req(None))).iter().any(|x| x.contains("mcp")));
    }

    #[test]
    fn every_run_turns_off_background_tasks_that_would_outlive_the_turn() {
        for b in [backend(), session()] {
            let cmd = b.command(&req(None));
            let set: Vec<_> = cmd.as_std().get_envs().filter(|(k, _)| *k == NO_BACKGROUND_ENV).collect();
            assert_eq!(set.len(), 1);
            assert_eq!(set[0].1.and_then(|v| v.to_str()), Some("1"));
        }
    }

    #[test]
    fn effort_sets_the_thinking_budget_in_the_environment_once() {
        let mut b = backend();
        b.set_effort(crate::effort::Effort::Low);
        b.set_effort(crate::effort::Effort::High);
        let cmd = b.command(&req(None));
        let budgets: Vec<_> = cmd.as_std().get_envs().filter(|(k, _)| *k == "MAX_THINKING_TOKENS").collect();
        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].1.and_then(|v| v.to_str()), Some("31999"));
        assert!(backend().command(&req(None)).as_std().get_envs().all(|(k, _)| k != "MAX_THINKING_TOKENS"));
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

    const ALLOW_TURN: &str = fixture!("claude/permission_allow.jsonl");
    const DENY_TURN: &str = fixture!("claude/permission_deny.jsonl");
    const ALLOW_LINE: &str = fixture!("claude/control_response_allow.jsonl");
    const DENY_LINE: &str = fixture!("claude/control_response_deny.jsonl");
    const USER_LINE: &str = fixture!("claude/user_message.jsonl");

    fn session() -> ClaudeBackend {
        backend().in_session()
    }

    #[test]
    fn a_session_takes_a_message_mid_turn_and_reports_when_the_model_has_it() {
        let line = session().steer_line("also run clippy").unwrap();
        assert_eq!(json_of(&line), json!({"type": "user", "message": {"role": "user", "content": "also run clippy"}}));
        assert!(session().steerable());
        // Without the host protocol stdin is closed after the prompt: nothing can be added.
        assert!(backend().steer_line("x").is_none() && !backend().steerable());
        let replay = |content: &str| format!(r#"{{"type":"user","message":{{"role":"user","content":{content}}},"isReplay":true,"session_id":"s"}}"#);
        let steered = |l: String| backend().parse_line(&l).into_iter().map(|e| match e {
            Event::Steered(t) => t,
            other => panic!("{other:?}"),
        }).collect::<Vec<_>>();
        assert_eq!(steered(replay(r#""also run clippy""#)), ["also run clippy"]);
        assert_eq!(steered(replay(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#)), ["a\nb"]);
        // A tool result is still a tool result.
        let result = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#;
        assert!(matches!(backend().parse_line(result).as_slice(), [Event::ToolResult { ok: true, .. }]));
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

    /// One golden case: `=== name`, then the arguments as a JSON array on one line. The
    /// `--setting-sources` value depends on the developer's trust settings, so it is masked.
    fn golden_case(name: &str, cmd: &Command) -> String {
        let mut a = args(cmd);
        if let Some(i) = a.iter().position(|x| x == "--setting-sources") {
            a[i + 1] = "<SOURCES>".into();
        }
        format!("=== {name}\n{}\n", serde_json::to_string(&a).unwrap())
    }

    #[test]
    fn golden_claude_args() {
        const MASKED: &str = "# masked: the value after --setting-sources (<SOURCES>, from the developer's trust settings)";
        const GOLDEN: &str = golden!("backend/claude_args.txt");
        let with_tools = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
        let mut mcp = session();
        mcp.attach_mcp(&attachment());
        let actual = [
            golden_case("fresh", &backend().command(&req(None))),
            golden_case("resumed", &with_tools.command(&req(Some("sess-1")))),
            golden_case("session_with_mcp", &mcp.command(&req(None))),
        ]
        .concat();
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn golden_deepseek_args() {
        const MASKED: &str = "# masked: the value after --setting-sources (<SOURCES>, from the developer's trust settings)";
        const GOLDEN: &str = golden!("backend/deepseek_args.txt");
        let deepseek = ClaudeBackend {
            provider: ProviderId::DeepSeek,
            env: vec![("ANTHROPIC_BASE_URL".into(), "https://deepseek.test/anthropic".into())],
            ..backend()
        };
        let actual = [
            golden_case("fresh", &deepseek.command(&req(None))),
            golden_case("resumed_in_session", &ClaudeBackend { interactive: true, ..deepseek }.command(&req(Some("sess-1")))),
        ]
        .concat();
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn golden_claude_control_response() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/claude_control_response.txt");
        let request = |input: Value, blocked_path: Option<&str>| PermissionRequest {
            id: "req-1".into(),
            tool: "Write".into(),
            summary: "unused".into(),
            input,
            blocked_path: blocked_path.map(String::from),
        };
        let full = request(json!({ "path": "/tmp/p/x.txt", "content": "a \"b\"\n", "nested": { "z": 1, "a": [1e2, 2.0] } }), Some("/tmp/p/x.txt"));
        let bare = request(json!({}), None);
        let line = |r: &PermissionRequest, d: Decision| session().permission_response(r, &d).unwrap();
        let actual = format!(
            "=== allow\n{}\n=== allow_always\n{}\n=== deny\n{}\n=== allow_empty_input\n{}\n",
            line(&full, Decision::Allow),
            line(&full, Decision::AllowAlways),
            line(&full, Decision::Deny { reason: "not here".into() }),
            line(&bare, Decision::Allow),
        );
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
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
            ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--setting-sources", "user,local", "--permission-mode", "acceptEdits",
             "--input-format", "stream-json", "--replay-user-messages", "--permission-prompt-tool", "stdio", "--disallowedTools", "AskUserQuestion",
             "--append-system-prompt", SESSION_NOTE]
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
        assert!(SESSION_NOTE.len() < 900, "{}", SESSION_NOTE.len());
        assert!(SESSION_NOTE.contains("jev-code") && SESSION_NOTE.contains("/permissions") && SESSION_NOTE.contains("Shift+Tab"));
        assert!(!SESSION_NOTE.contains('\n'), "no stray line breaks from the source layout");
        let a = args(&session().command(&req(Some("abc"))));
        let b = args(&session().command(&req(None)));
        let note = |v: &[String]| v[v.iter().position(|x| x == "--append-system-prompt").unwrap() + 1].clone();
        assert_eq!(note(&a), note(&b), "byte-identical every turn");
    }

    fn spec(p: ProviderId, id: &str) -> &'static crate::catalog::ModelSpec {
        crate::catalog::find(p, id).unwrap()
    }

    #[test]
    fn golden_mcp_config() {
        use crate::orchestrate::attachment;
        use std::path::Path;
        use ProviderId::Claude;
        let dir = std::env::temp_dir().join(format!("jev-golden-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let written = |name: &str, command: &str, args: &[&str]| {
            let mcp = McpAttachment {
                name: name.into(),
                command: command.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                config_path: dir.join("mcp.json"),
            };
            write_mcp_config(&mcp).unwrap();
            std::fs::read_to_string(&mcp.config_path).unwrap()
        };
        let real = attachment(Path::new("/opt/jev/bin/delegate_mcp"), Path::new("/work/app"), spec(Claude, "claude-opus-5-5"), &dir, 1);
        write_mcp_config(&real).unwrap();
        let typical = std::fs::read_to_string(&real.config_path).unwrap();
        let actual = format!(
            "## typical: the delegate server as `attachment` builds it\n{typical}\n## unusual: no arguments, spaces, quotes and non-ASCII text\n{}\n",
            written("j\"ev é", "/opt/my tools/é \\ delegate", &[]),
        );
        let _ = std::fs::remove_dir_all(&dir);
        let golden = golden!("backend/mcp_config.txt");
        assert_eq!(golden.split_once('\n').unwrap().1, actual);
    }
}
