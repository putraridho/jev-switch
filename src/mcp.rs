//! MCP server discovery and status across providers.
//!
//! Gathers MCP servers configured for Claude Code (`~/.claude.json` and local `.mcp.json`),
//! Codex (`~/.codex/config.toml`), Gemini / Antigravity (`~/.gemini/config/mcp_config.json`),
//! and `jev-code`'s internal worker delegation tool (`delegate_mcp`).

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpScope {
    Orchestrate,
    Project,
    Claude,
    Codex,
    Gemini,
}

impl std::fmt::Display for McpScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orchestrate => write!(f, "orchestrate"),
            Self::Project => write!(f, "project"),
            Self::Claude => write!(f, "claude"),
            Self::Codex => write!(f, "codex"),
            Self::Gemini => write!(f, "gemini"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    Http,
    Sse,
    Stdio,
}

impl std::fmt::Display for McpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http => write!(f, "http"),
            Self::Sse => write!(f, "sse"),
            Self::Stdio => write!(f, "stdio"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRow {
    pub name: String,
    pub scope: McpScope,
    pub transport: McpTransport,
    pub target: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpRawEntry {
    #[serde(default, rename = "type")]
    transport_type: Option<String>,
    url: Option<String>,
    #[serde(rename = "serverUrl")]
    server_url: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

impl McpRawEntry {
    fn transport(&self) -> McpTransport {
        match self.transport_type.as_deref() {
            Some("http") => McpTransport::Http,
            Some("sse") => McpTransport::Sse,
            Some("stdio") => McpTransport::Stdio,
            _ => {
                if self.url.is_some() || self.server_url.is_some() {
                    McpTransport::Http
                } else {
                    McpTransport::Stdio
                }
            }
        }
    }

    fn target(&self) -> String {
        if let Some(u) = self.url.as_deref().or(self.server_url.as_deref()) {
            u.to_string()
        } else if let Some(cmd) = self.command.as_deref() {
            if self.args.is_empty() {
                cmd.to_string()
            } else {
                format!("{cmd} {}", self.args.join(" "))
            }
        } else {
            String::new()
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpRawEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ClaudeProjectConfig {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpRawEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ClaudeConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpRawEntry>,
    #[serde(default)]
    projects: BTreeMap<String, ClaudeProjectConfig>,
}

/// Parses the `[mcp_servers.<name>]` sections of a Codex `config.toml`.
pub fn parse_codex_toml(content: &str) -> Vec<McpRow> {
    let mut out = Vec::new();
    let mut current_server: Option<String> = None;
    let mut current_url: Option<String> = None;
    let mut current_command: Option<String> = None;
    let mut current_args: Vec<String> = Vec::new();

    let flush = |out: &mut Vec<McpRow>,
                 server: &mut Option<String>,
                 url: &mut Option<String>,
                 cmd: &mut Option<String>,
                 args: &mut Vec<String>| {
        if let Some(name) = server.take() {
            let (transport, target) = if let Some(u) = url.take() {
                (McpTransport::Http, u)
            } else if let Some(c) = cmd.take() {
                let target = if args.is_empty() {
                    c
                } else {
                    format!("{c} {}", args.join(" "))
                };
                args.clear();
                (McpTransport::Stdio, target)
            } else {
                (McpTransport::Stdio, String::new())
            };
            out.push(McpRow {
                name,
                scope: McpScope::Codex,
                transport,
                target,
            });
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = line[1..line.len() - 1].trim();
            if let Some(rest) = section.strip_prefix("mcp_servers.") {
                // Ignore sub-tables like [mcp_servers.name.tools...]
                if !rest.contains('.') {
                    flush(&mut out, &mut current_server, &mut current_url, &mut current_command, &mut current_args);
                    current_server = Some(rest.trim_matches('"').trim_matches('\'').to_string());
                }
            } else {
                flush(&mut out, &mut current_server, &mut current_url, &mut current_command, &mut current_args);
            }
            continue;
        }

        if current_server.is_some() {
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"').trim_matches('\'');
                match key {
                    "url" => current_url = Some(val.to_string()),
                    "command" => current_command = Some(val.to_string()),
                    _ => {}
                }
            }
        }
    }
    flush(&mut out, &mut current_server, &mut current_url, &mut current_command, &mut current_args);
    out
}

/// Discovers all configured MCP servers across providers.
pub fn discover(project_dir: &Path, home: Option<&Path>, orchestrate_on: bool) -> Vec<McpRow> {
    let mut rows = Vec::new();

    // 1. Orchestration delegate tool
    if orchestrate_on {
        rows.push(McpRow {
            name: "jev".to_string(),
            scope: McpScope::Orchestrate,
            transport: McpTransport::Stdio,
            target: "delegate_mcp (worker delegation)".to_string(),
        });
    }

    // 2. Project local .mcp.json
    let project_mcp_json = project_dir.join(".mcp.json");
    if let Ok(content) = std::fs::read_to_string(&project_mcp_json) {
        if let Ok(cfg) = serde_json::from_str::<McpConfigFile>(&content) {
            for (name, entry) in cfg.mcp_servers {
                rows.push(McpRow {
                    name,
                    scope: McpScope::Project,
                    transport: entry.transport(),
                    target: entry.target(),
                });
            }
        }
    }

    // 3. Claude Code: ~/.claude.json
    if let Some(home) = home {
        let claude_json = home.join(".claude.json");
        if let Ok(content) = std::fs::read_to_string(&claude_json) {
            if let Ok(cfg) = serde_json::from_str::<ClaudeConfigFile>(&content) {
                for (name, entry) in cfg.mcp_servers {
                    rows.push(McpRow {
                        name,
                        scope: McpScope::Claude,
                        transport: entry.transport(),
                        target: entry.target(),
                    });
                }
                // Check project entry in ~/.claude.json
                let project_key = project_dir.to_string_lossy();
                if let Some(proj) = cfg.projects.get(project_key.as_ref()) {
                    for (name, entry) in &proj.mcp_servers {
                        if !rows.iter().any(|r| r.scope == McpScope::Claude && r.name == *name) {
                            rows.push(McpRow {
                                name: name.clone(),
                                scope: McpScope::Claude,
                                transport: entry.transport(),
                                target: entry.target(),
                            });
                        }
                    }
                }
            }
        }

        // 4. Codex: ~/.codex/config.toml
        let codex_toml = home.join(".codex/config.toml");
        if let Ok(content) = std::fs::read_to_string(&codex_toml) {
            rows.extend(parse_codex_toml(&content));
        }

        // 5. Gemini / Antigravity: ~/.gemini/config/mcp_config.json
        let gemini_json = home.join(".gemini/config/mcp_config.json");
        if let Ok(content) = std::fs::read_to_string(&gemini_json) {
            if let Ok(cfg) = serde_json::from_str::<McpConfigFile>(&content) {
                for (name, entry) in cfg.mcp_servers {
                    rows.push(McpRow {
                        name,
                        scope: McpScope::Gemini,
                        transport: entry.transport(),
                        target: entry.target(),
                    });
                }
            }
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_toml_extracts_servers_and_skips_subtables() {
        let toml = r#"
model = "gpt-6-astra"

[mcp_servers.remote]
url = "https://example.com/mcp"

[mcp_servers.remote.tools.foo]
approval_mode = "approve"

[mcp_servers.local]
command = "npx"

[other_section]
key = "value"
"#;
        let rows = parse_codex_toml(toml);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "remote");
        assert_eq!(rows[0].scope, McpScope::Codex);
        assert_eq!(rows[0].transport, McpTransport::Http);
        assert_eq!(rows[0].target, "https://example.com/mcp");

        assert_eq!(rows[1].name, "local");
        assert_eq!(rows[1].scope, McpScope::Codex);
        assert_eq!(rows[1].transport, McpTransport::Stdio);
        assert_eq!(rows[1].target, "npx");
    }

    #[test]
    fn discover_with_orchestrate_adds_delegate() {
        let rows = discover(Path::new("/tmp"), None, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "jev");
        assert_eq!(rows[0].scope, McpScope::Orchestrate);
    }
}
