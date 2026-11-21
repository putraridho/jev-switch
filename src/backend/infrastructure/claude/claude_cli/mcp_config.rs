//! The file `--mcp-config` reads.

use serde_json::{json, Value};

use crate::backend::McpAttachment;

/// The JSON file `--mcp-config` reads: one stdio server. Pure so the shape is a test.
pub(super) fn mcp_config_json(mcp: &McpAttachment) -> Value {
    json!({ "mcpServers": { mcp.name.as_str(): { "command": mcp.command, "args": mcp.args } } })
}

/// Write the JSON file Claude Code's `--mcp-config` reads. Harmless for other CLIs.
pub fn write_mcp_config(mcp: &McpAttachment) -> std::io::Result<()> {
    let json = mcp_config_json(mcp);
    std::fs::write(&mcp.config_path, json.to_string())
}
