//! `/mcp`: the MCP servers the providers are configured with. Interactive by default (the
//! list of [`crate::tui::views::mcp_picker`]), the plain table with `/mcp list` or without
//! a terminal, and `/mcp login <server>` to sign in with the provider's own CLI.

use std::io::IsTerminal;
use std::path::Path;

use crate::backend::infrastructure::mcp_discovery::{self, McpRow, McpScope};
use crate::backend::infrastructure::{mcp_health_check, mcp_server};
use crate::shared::infrastructure::env_file as envfile;
use crate::tui::command::McpCmd;
use crate::tui::render as uifmt;
use crate::tui::repl::session::Repl;
use crate::tui::tty::terminal_guard as terminal;

impl Repl {
    /// `/mcp [list | login [<server>]]`.
    pub(super) fn mcp(&mut self, cmd: McpCmd) {
        let interactive = self.paging && std::io::stdin().is_terminal();
        match cmd {
            McpCmd::Browse if interactive => self.mcp_browse(),
            McpCmd::Browse | McpCmd::List => {
                let rows = self.mcp_rows();
                self.page_out(&uifmt::mcp_text(&rows, self.out_style()));
            }
            McpCmd::Login(server) => self.mcp_login(&server),
        }
    }

    fn mcp_rows(&self) -> Vec<McpRow> {
        mcp_discovery::discover(self.engine.dir(), self.home.as_deref().map(Path::new), self.engine.orchestrate())
    }

    /// The list, its status dots filled in by checks running while it is open; a login
    /// chosen on a server's card runs once the list is closed.
    fn mcp_browse(&mut self) {
        let style = self.out_style();
        let servers = mcp_server::group(&self.mcp_rows());
        if servers.is_empty() {
            return self.page_out(&uifmt::mcp_text(&[], style));
        }
        let answers = mcp_health_check::check_in_background(&servers);
        let chosen = self.console.with_terminal(|| crate::tui::views::mcp_picker::run(servers, answers, style, terminal::rows(), terminal::width()));
        if let Some((server, scope)) = chosen {
            self.mcp_login_with(&server, scope);
        }
    }

    /// `/mcp login <server>`: with the first provider that can sign in to it.
    fn mcp_login(&mut self, server: &str) {
        let style = self.out_style();
        let servers = mcp_server::group(&self.mcp_rows());
        let Some(found) = servers.iter().find(|s| s.name == server) else {
            let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
            let known = if names.is_empty() { String::new() } else { format!(" Configured: {}.", names.join(", ")) };
            self.console.out(&format!("{} Server `{server}` is not configured in any provider.{known}", style.yellow("⚠")));
            return;
        };
        let Some(&scope) = found.login_scopes().first() else {
            let scopes: Vec<String> = found.presences.iter().map(|p| p.scope.to_string()).collect();
            let why = format!("`{server}` ({}, under {}) has no login: only HTTP servers configured for claude or codex do.", found.transport, scopes.join(", "));
            self.console.out(&format!("  {}", style.dim(&why)));
            return;
        };
        self.mcp_login_with(server, scope);
    }

    /// Run `<provider> mcp login <server>` in the terminal.
    fn mcp_login_with(&mut self, server: &str, scope: McpScope) {
        let style = self.out_style();
        let bin = match scope {
            McpScope::Claude => envfile::lookup("CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            McpScope::Codex => envfile::lookup("CODEX_BIN").unwrap_or_else(|| "codex".into()),
            other => {
                self.console.out(&format!("  {}", style.dim(&format!("{other} has no MCP login command."))));
                return;
            }
        };
        self.console.out(&format!("  {}", style.dim(&format!("Logging in to `{server}` via `{bin} mcp login`..."))));
        let status = self.console.with_terminal(|| std::process::Command::new(&bin).args(["mcp", "login", server]).status());
        // Whatever happened, the kept health answers may be wrong now.
        mcp_health_check::forget_checks();
        match status {
            Ok(s) if s.success() => self.console.out(&format!("{} Signed in to `{server}`.", style.green("✓"))),
            Ok(s) => self.console.out(&format!("{} Login failed with exit code: {:?}", style.yellow("⚠"), s.code())),
            Err(e) => self.console.out(&format!("{} Failed to run `{bin}`: {e}", style.red("✗"))),
        }
    }
}
