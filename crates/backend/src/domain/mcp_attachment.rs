//! An MCP server attached to one turn.

use std::path::PathBuf;

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
