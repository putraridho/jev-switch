//! Claude Code history files: typed readers for the fields used.

use serde::Deserialize;

use super::turns::{ is_injected, tool_summary, TurnBuilder };
use jev_backend::ProviderId;
use crate::domain::turn_record::TurnRecord;

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

pub(super) fn claude_turns(text: &str) -> Vec<TurnRecord> {
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
