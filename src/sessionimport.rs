//! Bring a conversation from Claude Code or Codex into jev-code: read their history files
//! (JSON lines) into the turns a jev session holds, so `/resume` can continue it on any
//! provider. Typed readers for exactly the fields used; a line that does not fit is skipped,
//! so a format change loses detail, never the import.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;

use crate::backend::ProviderId;
use crate::handoff::{self, SavedSession, Sessions, TurnRecord};

/// Where a history comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ClaudeCode,
    Codex,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// One history file that belongs to the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub path: PathBuf,
    pub modified_ms: u64,
}

/// Most histories listed at once.
const MAX_FOUND: usize = 30;
/// Most Codex files looked into (their folder is shared by every project).
const MAX_CODEX_SCANNED: usize = 300;

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

/// Claude Code keeps a project's histories in `~/.claude/projects/<path with every
/// non-alphanumeric character turned into "-">/`.
pub fn claude_project_dir(home: &Path, project: &Path) -> PathBuf {
    let key = handoff::project_key(project);
    let encoded: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    home.join(".claude").join("projects").join(encoded)
}

/// The histories of `project` in `home`'s Claude Code and Codex folders, newest first.
pub fn discover(home: &Path, project: &Path) -> Vec<Found> {
    let mut found: Vec<Found> = jsonl_files(&claude_project_dir(home, project))
        .into_iter()
        .map(|path| Found { source: Source::ClaudeCode, modified_ms: modified_ms(&path), path })
        .collect();
    let key = handoff::project_key(project);
    let mut rollouts = codex_rollouts(&home.join(".codex").join("sessions"));
    rollouts.sort();
    rollouts.reverse();
    for path in rollouts.into_iter().take(MAX_CODEX_SCANNED) {
        if codex_cwd(&path).is_some_and(|cwd| handoff::project_key(Path::new(&cwd)) == key) {
            found.push(Found { source: Source::Codex, modified_ms: modified_ms(&path), path });
        }
    }
    found.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    found.truncate(MAX_FOUND);
    found
}

/// `sessions/YYYY/MM/DD/rollout-*.jsonl`, found without knowing the date folders.
fn codex_rollouts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for path in std::fs::read_dir(&dir).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|x| x == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

fn codex_cwd(path: &Path) -> Option<String> {
    let first = std::fs::read_to_string(path).ok()?.lines().next()?.to_string();
    serde_json::from_str::<CodexLine>(&first).ok()?.payload?.cwd
}

/// What a history file holds, as jev turns.
pub fn load(found: &Found) -> Result<Vec<TurnRecord>, String> {
    let text = std::fs::read_to_string(&found.path).map_err(|e| e.to_string())?;
    let turns = match found.source {
        Source::ClaudeCode => claude_turns(&text),
        Source::Codex => codex_turns(&text),
    };
    if turns.is_empty() {
        return Err("no conversation found in that history".to_string());
    }
    Ok(turns)
}

/// The history as a saved session of `project`, last used when its file was last written.
pub fn to_saved(turns: Vec<TurnRecord>, project: &Path, saved_ms: u64) -> SavedSession {
    let mut sessions = Sessions::new();
    for turn in turns {
        sessions.push_unseen(turn);
    }
    SavedSession { project: handoff::project_key(project), saved_ms, sessions, current: None }
}

// ------------------------------------------------------------------ turns

/// Builds turns from a stream of user prompts, answers and tool calls.
struct TurnBuilder {
    provider: ProviderId,
    done: Vec<TurnRecord>,
    open: Option<TurnRecord>,
}

impl TurnBuilder {
    fn new(provider: ProviderId) -> Self {
        Self { provider, done: Vec::new(), open: None }
    }

    fn prompt(&mut self, text: &str) {
        self.close();
        self.open = Some(TurnRecord { prompt: text.trim().to_string(), provider: self.provider, model: String::new(), reply: String::new(), tools: Vec::new(), ok: true });
    }

    fn answer(&mut self, text: &str, model: Option<&str>) {
        if let Some(t) = self.open.as_mut() {
            if !text.trim().is_empty() {
                t.reply = text.trim().to_string();
            }
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                t.model = m.to_string();
            }
        }
    }

    fn tool(&mut self, summary: String) {
        if let Some(t) = self.open.as_mut() {
            t.tools.push(summary);
        }
    }

    fn close(&mut self) {
        self.done.extend(self.open.take());
    }

    fn finish(mut self) -> Vec<TurnRecord> {
        self.close();
        self.done
    }
}

/// A "user" message the person did not type: injected context and command wrappers.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty() || t.starts_with('<') || t.starts_with("Caveat:") || t.starts_with("[Request interrupted")
}

fn tool_summary(name: &str, arg: Option<&str>) -> String {
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name} {}", a.lines().next().unwrap_or_default()),
        None => name.to_string(),
    }
}

// ------------------------------------------------------------ Claude Code

#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    model: Option<String>,
    content: Option<ClaudeContent>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ClaudeContent {
    Text(String),
    Blocks(Vec<ClaudeBlock>),
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClaudeBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { name: String, #[serde(default)] input: ToolInput },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Default)]
struct ToolInput {
    file_path: Option<String>,
    path: Option<String>,
    command: Option<String>,
    pattern: Option<String>,
}

impl ToolInput {
    fn arg(&self) -> Option<&str> {
        [&self.file_path, &self.path, &self.command, &self.pattern].into_iter().flatten().map(String::as_str).next()
    }
}

fn claude_turns(text: &str) -> Vec<TurnRecord> {
    let mut turns = TurnBuilder::new(ProviderId::Claude);
    for line in text.lines().filter_map(|l| serde_json::from_str::<ClaudeLine>(l).ok()) {
        let Some(message) = line.message else { continue };
        match (line.kind.as_deref(), message.content) {
            (Some("user"), Some(content)) if !line.is_meta => {
                let typed = match content {
                    ClaudeContent::Text(t) => t,
                    ClaudeContent::Blocks(blocks) => blocks
                        .into_iter()
                        .filter_map(|b| match b {
                            ClaudeBlock::Text { text } => Some(text),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                if !is_injected(&typed) {
                    turns.prompt(&typed);
                }
            }
            (Some("assistant"), Some(ClaudeContent::Blocks(blocks))) => {
                for block in blocks {
                    match block {
                        ClaudeBlock::Text { text } => turns.answer(&text, message.model.as_deref()),
                        ClaudeBlock::ToolUse { name, input } => turns.tool(tool_summary(&name, input.arg())),
                        ClaudeBlock::Other => {}
                    }
                }
            }
            _ => {}
        }
    }
    turns.finish()
}

// ------------------------------------------------------------------ Codex

#[derive(Deserialize)]
struct CodexLine {
    payload: Option<CodexPayload>,
}

#[derive(Deserialize)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    role: Option<String>,
    content: Option<Vec<CodexPart>>,
    name: Option<String>,
    arguments: Option<String>,
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct CodexPart {
    text: Option<String>,
}

#[derive(Deserialize)]
struct CodexArguments {
    command: Option<CodexCommand>,
    input: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CodexCommand {
    Line(String),
    Argv(Vec<String>),
}

impl CodexPayload {
    fn text(&self) -> String {
        self.content.iter().flatten().filter_map(|p| p.text.as_deref()).collect::<Vec<_>>().join("\n")
    }

    /// `bash ls -la` for a shell call, `apply_patch src/a.rs` for a patch, else the name.
    fn tool_summary(&self) -> Option<String> {
        let name = self.name.as_deref()?;
        let args: Option<CodexArguments> = self.arguments.as_deref().and_then(|a| serde_json::from_str(a).ok());
        if let Some(patch) = args.as_ref().and_then(|a| a.input.as_deref()) {
            let file = patch.lines().find_map(|l| {
                ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|p| l.strip_prefix(p))
            });
            return Some(tool_summary("apply_patch", file));
        }
        match args.and_then(|a| a.command) {
            Some(CodexCommand::Line(c)) => Some(tool_summary("bash", Some(&c))),
            // `["bash", "-lc", "cargo test"]`: the script is the last word.
            Some(CodexCommand::Argv(argv)) => Some(tool_summary("bash", argv.last().map(String::as_str))),
            None => Some(tool_summary(name, None)),
        }
    }
}

fn codex_turns(text: &str) -> Vec<TurnRecord> {
    let mut turns = TurnBuilder::new(ProviderId::Codex);
    for payload in text.lines().filter_map(|l| serde_json::from_str::<CodexLine>(l).ok()).filter_map(|l| l.payload) {
        match (payload.kind.as_deref(), payload.role.as_deref()) {
            (Some("message"), Some("user")) => {
                let typed = payload.text();
                if !is_injected(&typed) {
                    turns.prompt(&typed);
                }
            }
            (Some("message"), Some("assistant")) => turns.answer(&payload.text(), None),
            (Some("function_call" | "custom_tool_call" | "local_shell_call"), _) => {
                if let Some(summary) = payload.tool_summary() {
                    turns.tool(summary);
                }
            }
            _ => {}
        }
    }
    turns.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-imp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const CLAUDE: &str = r#"{"type":"summary","summary":"x"}
{"type":"user","message":{"role":"user","content":"fix the parser"}}
{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: meta"}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Looking."},{"type":"tool_use","id":"1","name":"Edit","input":{"file_path":"/p/src/parser.rs","old_string":"a"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"1","content":"ok"}]}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"text","text":"Fixed it."}]}}
{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"now add tests"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"2","name":"Bash","input":{"command":"cargo test"}},{"type":"text","text":"Tests pass."}]}}
this line is not json
"#;

    const CODEX: &str = r#"{"type":"session_meta","payload":{"id":"x","cwd":"/p"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>..</environment_context>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list the files"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls -la\"]}"}}
{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","arguments":"{\"input\":\"*** Begin Patch\\n*** Update File: src/a.rs\\n@@\"}"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Here they are."}]}}
{"type":"event_msg","payload":{"type":"token_count"}}
"#;

    fn prompts(turns: &[TurnRecord]) -> Vec<&str> {
        turns.iter().map(|t| t.prompt.as_str()).collect()
    }

    #[test]
    fn claude_code_history_becomes_turns_with_tools_and_the_final_answer() {
        let turns = claude_turns(CLAUDE);
        assert_eq!(prompts(&turns), ["fix the parser", "now add tests"]);
        assert_eq!(turns[0].reply, "Fixed it.");
        assert_eq!(turns[0].model, "claude-sonnet-5-5");
        assert_eq!(turns[0].tools, ["Edit /p/src/parser.rs"]);
        assert_eq!(turns[1].tools, ["Bash cargo test"]);
        assert_eq!(turns[1].reply, "Tests pass.");
        assert!(turns.iter().all(|t| t.provider == ProviderId::Claude && t.ok));
        // Tools in jev's format are understood by the hand-off.
        assert_eq!(handoff::files_touched(&turns[0].tools), ["/p/src/parser.rs"]);
        assert_eq!(handoff::commands_run(&turns[1].tools), ["cargo test"]);
    }

    #[test]
    fn codex_history_skips_injected_context_and_names_shell_and_patch_calls() {
        let turns = codex_turns(CODEX);
        assert_eq!(prompts(&turns), ["list the files"]);
        assert_eq!(turns[0].tools, ["bash ls -la", "apply_patch src/a.rs"]);
        assert_eq!(turns[0].reply, "Here they are.");
        assert_eq!(turns[0].provider, ProviderId::Codex);
        assert_eq!(handoff::files_touched(&turns[0].tools), ["src/a.rs"]);
    }

    #[test]
    fn discovery_finds_this_projects_histories_in_both_tools() {
        let home = temp("discover");
        let project = home.join("work").join("my.app");
        std::fs::create_dir_all(&project).unwrap();
        let claude = claude_project_dir(&home, &project);
        assert!(claude.to_string_lossy().ends_with("-work-my-app"), "{claude:?}");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("a.jsonl"), CLAUDE).unwrap();
        std::fs::write(claude.join("notes.txt"), "no").unwrap();
        let day = home.join(".codex/sessions/2026/11/17");
        std::fs::create_dir_all(&day).unwrap();
        let mine = CODEX.replace("\"/p\"", &serde_json::to_string(&handoff::project_key(&project)).unwrap());
        std::fs::write(day.join("rollout-1.jsonl"), mine).unwrap();
        std::fs::write(day.join("rollout-2.jsonl"), CODEX).unwrap(); // another project's
        let found = discover(&home, &project);
        let mut sources: Vec<Source> = found.iter().map(|f| f.source).collect();
        sources.sort_by_key(|s| s.label());
        assert_eq!(sources, [Source::ClaudeCode, Source::Codex]);
        let codex = found.iter().find(|f| f.source == Source::Codex).unwrap();
        assert!(codex.path.ends_with("rollout-1.jsonl"));
        assert_eq!(load(codex).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_history_is_an_error_and_a_loaded_one_resumes() {
        let dir = temp("load");
        let empty = dir.join("e.jsonl");
        std::fs::write(&empty, "{\"type\":\"summary\"}\n").unwrap();
        let found = Found { source: Source::ClaudeCode, path: empty, modified_ms: 5 };
        assert!(load(&found).is_err());
        let saved = to_saved(claude_turns(CLAUDE), &dir, 5);
        assert_eq!(saved.sessions.turns(), 2);
        // Nobody has lived these turns, so the first provider to run is handed them.
        assert_eq!(saved.sessions.missed_turns(ProviderId::Gemini), 2);
    }
}
