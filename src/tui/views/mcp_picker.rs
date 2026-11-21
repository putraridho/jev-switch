//! The `/mcp` list: every configured MCP server once, with a status dot that fills in as
//! the providers answer. Arrow keys move, typing narrows the list, Enter or → opens a
//! server's card (where it points, how each provider sees it, and how to log in), and
//! ← or Esc goes back.
//!
//! The state, the key decoding and the drawing are plain functions of their input so they
//! are tested; only [`session::run`] touches the terminal.

mod detail_view;
mod input;
mod keys;
mod list_view;
pub mod session;
#[cfg(test)]
mod tests;

use crate::backend::infrastructure::mcp_discovery::McpScope;
use crate::backend::infrastructure::mcp_server::{McpHealth, McpServer};
use crate::tui::render::Style;
use keys::decode;

/// A key the picker understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Enter,
    /// Esc alone, or Ctrl-D.
    Cancel,
    /// Ctrl-C.
    Interrupt,
    Backspace,
    Char(char),
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// Log in to this server with this provider's CLI.
    Login {
        server: String,
        scope: McpScope,
    },
    Cancelled,
}

/// What a server's card offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Login(McpScope),
    Back,
}

/// Which screen is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    List,
    /// The card of the server at this index of the servers, with the action under the cursor.
    Detail {
        server: usize,
        action: usize,
    },
}

/// The servers, the search, the cursor and the screen.
pub struct McpPicker {
    servers: Vec<McpServer>,
    filter: String,
    /// The cursor, as a position among the servers the filter lets through.
    selected: usize,
    /// The first row of the page shown.
    offset: usize,
    page: usize,
    screen: Screen,
}

impl McpPicker {
    /// `page`: how many servers are shown at once.
    pub fn new(servers: Vec<McpServer>, page: usize) -> Self {
        Self {
            servers,
            filter: String::new(),
            selected: 0,
            offset: 0,
            page: page.max(1),
            screen: Screen::List,
        }
    }

    /// Indexes into the servers of those that match what was typed, in list order.
    fn visible(&self) -> Vec<usize> {
        (0..self.servers.len())
            .filter(|&i| matches(&self.servers[i], &self.filter))
            .collect()
    }

    fn actions(server: &McpServer) -> Vec<Action> {
        server
            .login_scopes()
            .into_iter()
            .map(Action::Login)
            .chain([Action::Back])
            .collect()
    }

    /// A health check answered: update that provider's view of the server.
    pub fn set_health(&mut self, server: &str, scope: McpScope, health: McpHealth) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.name == server) {
            s.set_health(scope, health);
        }
    }

    /// Whether any answer is still awaited.
    fn checking(&self) -> bool {
        self.servers
            .iter()
            .any(|s| s.presences.iter().any(|p| p.health == McpHealth::Checking))
    }

    pub fn press(&mut self, key: Key) -> Outcome {
        match self.screen {
            Screen::List => self.press_list(key),
            Screen::Detail { server, action } => self.press_detail(key, server, action),
        }
    }
    /// The lines to draw for the screen that is up.
    pub fn render(&self, width: usize, style: Style) -> Vec<String> {
        let width = width.max(24);
        match self.screen {
            Screen::List => self.render_list(width, style),
            Screen::Detail { server, action } => self.render_detail(server, action, width, style),
        }
    }
}

/// The status dot of a state.
fn dot(health: &McpHealth, style: Style) -> String {
    match health {
        McpHealth::Connected => style.green("●"),
        McpHealth::NeedsLogin => style.yellow("◐"),
        McpHealth::Failed(_) => style.red("✗"),
        McpHealth::Checking => style.dim("◌"),
        McpHealth::Disabled => style.dim("○"),
        McpHealth::Unknown => style.dim("·"),
    }
}

/// A line of text with nothing in it that could move the cursor.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Does the server match what was typed: every word, in any case, is in its name, target,
/// transport or providers.
fn matches(s: &McpServer, filter: &str) -> bool {
    let mut hay = format!("{} {} {}", s.name, s.target, s.transport).to_lowercase();
    for p in &s.presences {
        hay.push(' ');
        hay.push_str(&p.scope.to_string());
    }
    filter
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// How many servers fit on a screen of `rows` rows (about nine for the chrome), at most 15.
pub fn page_for(rows: Option<usize>) -> usize {
    rows.map_or(10, |r| r.saturating_sub(9).clamp(1, 15))
}
