//! The `--mcp-config` file.

use super::*;
use crate::infrastructure::claude::claude_cli::mcp_config::mcp_config_json;

#[test]
fn mcp_config_file_runs_the_server_command() {
    let v = mcp_config_json(&attachment());
    assert_eq!(v["mcpServers"]["jev"]["command"], "/bin/delegate_mcp");
    assert_eq!(v["mcpServers"]["jev"]["args"], json!(["--dir", "/repo"]));
}
