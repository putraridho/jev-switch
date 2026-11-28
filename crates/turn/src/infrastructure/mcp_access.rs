//! Translate provider MCP configuration into routing facts.

use std::path::Path;
use crate::domain::mcp_access::McpAccess;

pub fn discover(dir: &Path) -> Vec<McpAccess> {
    use jev_backend::infrastructure::mcp_discovery::{discover, McpScope};
    use jev_shared::ProviderId;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    discover(dir, home.as_deref(), false).into_iter().map(|row| {
        let providers = match row.scope {
            McpScope::Claude | McpScope::Project => vec![ProviderId::Claude, ProviderId::DeepSeek],
            McpScope::Codex => vec![ProviderId::Codex],
            McpScope::Gemini => vec![ProviderId::Gemini],
            McpScope::Orchestrate => vec![],
        };
        crate::domain::mcp_access::McpAccess { name: row.name, providers }
    }).fold(Vec::<crate::domain::mcp_access::McpAccess>::new(), |mut servers, row| {
        if let Some(server) = servers.iter_mut().find(|s| s.name == row.name) {
            server.providers.extend(row.providers);
        } else {
            servers.push(row);
        }
        servers
    })
}
