//! Codex rollout files: finding them, the project they belong to, and their turns.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::turns::{ is_injected, tool_summary, TurnBuilder };
use crate::backend::ProviderId;
use crate::session::domain::turn_record::TurnRecord;

/// `sessions/YYYY/MM/DD/rollout-*.jsonl`, found without knowing the date folders.
pub(super) fn codex_rollouts(root: &Path) -> Vec<PathBuf> {
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


pub(super) fn codex_cwd(path: &Path) -> Option<String> {
    // Only the first line is wanted: do not read a multi-megabyte rollout to get it.
    let mut first = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(std::fs::File::open(path).ok()?), &mut first).ok()?;
    serde_json::from_str::<CodexLine>(first.trim_end()).ok()?.payload?.cwd
}

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

pub(super) fn codex_turns(text: &str) -> Vec<TurnRecord> {
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
