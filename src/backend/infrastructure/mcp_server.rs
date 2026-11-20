//! One MCP server as the person thinks of it: a name, configured under one or more
//! providers, each of which can see it in its own state. [`mcp_discovery`] finds one row
//! per provider config; [`group`] folds those rows into servers.
//!
//! [`mcp_discovery`]: super::mcp_discovery

use super::mcp_discovery::{McpRow, McpScope, McpTransport};

/// How one provider sees a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpHealth {
    /// The check has not answered yet.
    Checking,
    /// The provider connected to it.
    Connected,
    /// It needs an OAuth sign-in (`<provider> mcp login <name>`).
    NeedsLogin,
    /// The provider could not use it; the reason, as the provider said it.
    Failed(String),
    /// Configured but switched off in the provider's config.
    Disabled,
    /// Nothing checks this provider's servers (gemini, the project file, orchestrate).
    Unknown,
}

impl McpHealth {
    /// Which state wins when providers disagree: the one that needs the person most.
    fn urgency(&self) -> u8 {
        match self {
            Self::NeedsLogin => 6,
            Self::Failed(_) => 5,
            Self::Checking => 4,
            Self::Connected => 3,
            Self::Disabled => 2,
            Self::Unknown => 1,
        }
    }

    /// A short word for the state, as shown in lists.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Connected => "connected",
            Self::NeedsLogin => "needs login",
            Self::Failed(_) => "failed",
            Self::Disabled => "disabled",
            Self::Unknown => "not checked",
        }
    }
}

/// One provider's view of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPresence {
    pub scope: McpScope,
    pub health: McpHealth,
}

/// A server, with every provider it is configured under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    /// From the first provider config that names it.
    pub transport: McpTransport,
    pub target: String,
    /// In discovery order; one entry per provider.
    pub presences: Vec<McpPresence>,
}

impl McpServer {
    /// The state to show for the server as a whole: the most urgent of its providers'.
    pub fn health(&self) -> &McpHealth {
        self.presences.iter().map(|p| &p.health).max_by_key(|h| h.urgency()).unwrap_or(&McpHealth::Unknown)
    }

    /// The providers that can sign in to this server with `<provider> mcp login`.
    pub fn login_scopes(&self) -> Vec<McpScope> {
        if self.transport == McpTransport::Stdio {
            return Vec::new();
        }
        self.presences.iter().map(|p| p.scope).filter(|s| matches!(s, McpScope::Claude | McpScope::Codex)).collect()
    }

    /// Set what `scope` reports for this server; a scope it is not configured under is ignored.
    pub fn set_health(&mut self, scope: McpScope, health: McpHealth) {
        if let Some(p) = self.presences.iter_mut().find(|p| p.scope == scope) {
            p.health = health;
        }
    }
}

/// Fold per-provider rows into servers, in the order each name first appears. Claude and
/// Codex servers start as [`McpHealth::Checking`] (a check will answer), the rest as
/// [`McpHealth::Unknown`].
pub fn group(rows: &[McpRow]) -> Vec<McpServer> {
    let mut servers: Vec<McpServer> = Vec::new();
    for row in rows {
        let health = match row.scope {
            McpScope::Claude | McpScope::Codex => McpHealth::Checking,
            _ => McpHealth::Unknown,
        };
        let presence = McpPresence { scope: row.scope, health };
        match servers.iter_mut().find(|s| s.name == row.name) {
            Some(s) if !s.presences.iter().any(|p| p.scope == row.scope) => s.presences.push(presence),
            Some(_) => {}
            None => servers.push(McpServer { name: row.name.clone(), transport: row.transport, target: row.target.clone(), presences: vec![presence] }),
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, scope: McpScope, transport: McpTransport) -> McpRow {
        McpRow { name: name.into(), scope, transport, target: format!("https://{name}.example/mcp") }
    }

    #[test]
    fn rows_of_one_name_become_one_server() {
        let rows = [
            row("luna", McpScope::Claude, McpTransport::Http),
            row("pw", McpScope::Claude, McpTransport::Stdio),
            row("luna", McpScope::Codex, McpTransport::Http),
            row("luna", McpScope::Gemini, McpTransport::Http),
        ];
        let servers = group(&rows);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "luna");
        let scopes: Vec<McpScope> = servers[0].presences.iter().map(|p| p.scope).collect();
        assert_eq!(scopes, vec![McpScope::Claude, McpScope::Codex, McpScope::Gemini]);
        assert_eq!(servers[0].presences[2].health, McpHealth::Unknown);
        assert_eq!(servers[0].login_scopes(), vec![McpScope::Claude, McpScope::Codex]);
        assert!(servers[1].login_scopes().is_empty(), "a stdio server has no login");
    }

    #[test]
    fn the_most_urgent_provider_state_wins() {
        let mut s = group(&[row("luna", McpScope::Claude, McpTransport::Http), row("luna", McpScope::Codex, McpTransport::Http)]).remove(0);
        assert_eq!(s.health(), &McpHealth::Checking);
        s.set_health(McpScope::Claude, McpHealth::Connected);
        s.set_health(McpScope::Codex, McpHealth::NeedsLogin);
        assert_eq!(s.health(), &McpHealth::NeedsLogin);
        s.set_health(McpScope::Codex, McpHealth::Connected);
        assert_eq!(s.health(), &McpHealth::Connected);
        s.set_health(McpScope::Gemini, McpHealth::Failed("x".into()));
        assert_eq!(s.health(), &McpHealth::Connected, "an unconfigured scope is ignored");
    }
}
