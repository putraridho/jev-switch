//! The `--mcp-config` file.

use super::*;
use crate::backend::infrastructure::claude::claude_cli::mcp_config::{mcp_config_json, write_mcp_config};

#[test]
fn mcp_config_file_runs_the_server_command() {
    let v = mcp_config_json(&attachment());
    assert_eq!(v["mcpServers"]["jev"]["command"], "/bin/delegate_mcp");
    assert_eq!(v["mcpServers"]["jev"]["args"], json!(["--dir", "/repo"]));
}

#[test]
fn golden_mcp_config() {
    use crate::orchestration::domain::orchestration_gate::attachment;
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
