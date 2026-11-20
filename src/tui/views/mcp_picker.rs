//! The `/mcp` list: every configured MCP server once, with a status dot that fills in as
//! the providers answer. Arrow keys move, typing narrows the list, Enter or → opens a
//! server's card (where it points, how each provider sees it, and how to log in), and
//! ← or Esc goes back.
//!
//! The state, the key decoding and the drawing are plain functions of their input so they
//! are tested; only [`run`] touches the terminal.

use crate::backend::infrastructure::mcp_discovery::McpScope;
use crate::backend::infrastructure::mcp_server::{McpHealth, McpServer};
use crate::tui::render::{display_width, truncate_to_width, Style};

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
    Login { server: String, scope: McpScope },
    Cancelled,
}

/// The keys in one read from the terminal: an escape sequence arrives whole, typed or
/// pasted text arrives as many characters.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    match bytes {
        [] => vec![],
        [0x1b] => vec![Key::Cancel],
        [0x1b, b'[' | b'O', b'A'] => vec![Key::Up],
        [0x1b, b'[' | b'O', b'B'] => vec![Key::Down],
        [0x1b, b'[' | b'O', b'C'] => vec![Key::Right],
        [0x1b, b'[' | b'O', b'D'] => vec![Key::Left],
        [0x1b, b'[', b'5', b'~'] => vec![Key::PageUp],
        [0x1b, b'[', b'6', b'~'] => vec![Key::PageDown],
        // Any other escape sequence (Home, End, F-keys, ...) says nothing.
        [0x1b, ..] => vec![],
        _ => String::from_utf8_lossy(bytes).chars().filter_map(key_for_char).collect(),
    }
}

fn key_for_char(c: char) -> Option<Key> {
    match c {
        '\r' | '\n' => Some(Key::Enter),
        '\u{7f}' | '\u{8}' => Some(Key::Backspace),
        '\u{3}' => Some(Key::Interrupt),
        '\u{4}' => Some(Key::Cancel),
        '\u{10}' => Some(Key::Up),
        '\u{e}' => Some(Key::Down),
        c if c.is_control() => None,
        c => Some(Key::Char(c)),
    }
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
    Detail { server: usize, action: usize },
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
        Self { servers, filter: String::new(), selected: 0, offset: 0, page: page.max(1), screen: Screen::List }
    }

    /// Indexes into the servers of those that match what was typed, in list order.
    fn visible(&self) -> Vec<usize> {
        (0..self.servers.len()).filter(|&i| matches(&self.servers[i], &self.filter)).collect()
    }

    fn actions(server: &McpServer) -> Vec<Action> {
        server.login_scopes().into_iter().map(Action::Login).chain([Action::Back]).collect()
    }

    /// A health check answered: update that provider's view of the server.
    pub fn set_health(&mut self, server: &str, scope: McpScope, health: McpHealth) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.name == server) {
            s.set_health(scope, health);
        }
    }

    /// Whether any answer is still awaited.
    fn checking(&self) -> bool {
        self.servers.iter().any(|s| s.presences.iter().any(|p| p.health == McpHealth::Checking))
    }

    pub fn press(&mut self, key: Key) -> Outcome {
        match self.screen {
            Screen::List => self.press_list(key),
            Screen::Detail { server, action } => self.press_detail(key, server, action),
        }
    }

    fn press_list(&mut self, key: Key) -> Outcome {
        let shown = self.visible();
        let last = shown.len().saturating_sub(1);
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(last),
            Key::PageUp => self.selected = self.selected.saturating_sub(self.page),
            Key::PageDown => self.selected = (self.selected + self.page).min(last),
            Key::Enter | Key::Right => {
                if let Some(&server) = shown.get(self.selected) {
                    self.screen = Screen::Detail { server, action: 0 };
                }
            }
            // Esc first clears a search, then leaves.
            Key::Cancel if !self.filter.is_empty() => {
                self.filter.clear();
                self.selected = 0;
            }
            Key::Cancel | Key::Interrupt => return Outcome::Cancelled,
            Key::Left => {}
            Key::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            Key::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
        }
        self.keep_cursor_on_page();
        Outcome::Continue
    }

    fn press_detail(&mut self, key: Key, server: usize, action: usize) -> Outcome {
        let actions = Self::actions(&self.servers[server]);
        let last = actions.len().saturating_sub(1);
        match key {
            Key::Up => self.screen = Screen::Detail { server, action: action.saturating_sub(1) },
            Key::Down => self.screen = Screen::Detail { server, action: (action + 1).min(last) },
            Key::Enter => match actions.get(action) {
                Some(Action::Login(scope)) => return Outcome::Login { server: self.servers[server].name.clone(), scope: *scope },
                Some(Action::Back) | None => self.screen = Screen::List,
            },
            Key::Left | Key::Cancel | Key::Backspace => self.screen = Screen::List,
            Key::Interrupt => return Outcome::Cancelled,
            Key::Right | Key::PageUp | Key::PageDown | Key::Char(_) => {}
        }
        Outcome::Continue
    }

    fn keep_cursor_on_page(&mut self) {
        let count = self.visible().len();
        if count <= self.page {
            self.offset = 0;
        } else if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.page {
            self.offset = self.selected + 1 - self.page;
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

    fn render_list(&self, width: usize, style: Style) -> Vec<String> {
        let shown = self.visible();
        let mut out = vec![style.bold("MCP servers"), String::new()];
        let search = if self.filter.is_empty() { style.dim("type to search") } else { truncate_to_width(&clean(&self.filter), width.saturating_sub(12)) };
        out.push(format!("  {} {search}", style.dim("Search:")));
        out.push(String::new());
        if shown.is_empty() {
            out.push(format!("  {}", style.dim("No server matches.")));
        }
        // Names get at most half the line after the marker and dot, so the providers show too.
        let name_cap = (width.saturating_sub(6) / 2).clamp(4, 28);
        let name_col = shown.iter().map(|&i| display_width(&clean(&self.servers[i].name))).max().unwrap_or(0).clamp(4, name_cap);
        for (pos, &i) in shown.iter().enumerate().skip(self.offset).take(self.page) {
            out.push(self.list_row(&self.servers[i], pos == self.selected, name_col, width, style));
        }
        if shown.len() > self.page {
            let end = (self.offset + self.page).min(shown.len());
            out.push(style.dim(&format!("  {}-{} of {}", self.offset + 1, end, shown.len())));
        }
        out.push(String::new());
        if self.checking() {
            out.push(style.dim("  checking servers…"));
        }
        out.push(style.dim(&truncate_to_width("  ↑↓ move · Enter/→ open · Esc close · type to search", width)));
        out
    }

    /// `❯ ● name   http · claude · codex  needs login`, cut to the width.
    fn list_row(&self, s: &McpServer, selected: bool, name_col: usize, width: usize, style: Style) -> String {
        let marker = if selected { style.cyan("❯") } else { " ".to_string() };
        let name = truncate_to_width(&clean(&s.name), name_col);
        let pad = " ".repeat(name_col.saturating_sub(display_width(&name)) + 2);
        let name = if selected { style.bold(&name) } else { name };
        let scopes: Vec<String> = s.presences.iter().map(|p| p.scope.to_string()).collect();
        let note = match s.health() {
            McpHealth::NeedsLogin => "needs login",
            McpHealth::Failed(_) => "failed",
            _ => "",
        };
        // marker, space, dot, space, name, pad
        let used = 4 + name_col + 2;
        let room = width.saturating_sub(used);
        let meta_text = format!("{} · {}", s.transport, scopes.join(" · "));
        let (meta, note) = if note.is_empty() {
            (truncate_to_width(&meta_text, room), String::new())
        } else {
            let meta = truncate_to_width(&meta_text, room.saturating_sub(note.len() + 2));
            let left = room.saturating_sub(display_width(&meta) + 2);
            (meta, truncate_to_width(note, left))
        };
        let note = match s.health() {
            McpHealth::NeedsLogin if !note.is_empty() => format!("  {}", style.yellow(&note)),
            McpHealth::Failed(_) if !note.is_empty() => format!("  {}", style.red(&note)),
            _ => String::new(),
        };
        format!("{marker} {} {name}{pad}{}{note}", dot(s.health(), style), style.dim(&meta))
    }

    fn render_detail(&self, server: usize, action: usize, width: usize, style: Style) -> Vec<String> {
        let s = &self.servers[server];
        let fit = |text: &str, indent: usize| truncate_to_width(&clean(text), width.saturating_sub(indent));
        let mut out = vec![style.bold(&fit(&s.name, 0)), String::new()];
        out.push(format!("  {} {}", style.dim("Target:   "), fit(&s.target, 13)));
        out.push(format!("  {} {}", style.dim("Transport:"), s.transport));
        out.push(String::new());
        out.push(format!("  {}", style.dim("Providers")));
        let scope_col = s.presences.iter().map(|p| p.scope.to_string().len()).max().unwrap_or(0);
        for p in &s.presences {
            let scope = format!("{:<scope_col$}", p.scope.to_string());
            let label = p.health.label();
            let mut line = format!("    {} {scope}  {label}", dot(&p.health, style));
            if let McpHealth::Failed(why) = &p.health {
                let used = 4 + 2 + scope_col + 2 + label.len() + 3;
                line.push_str(&style.dim(&format!(" — {}", fit(why, used))));
            }
            out.push(line);
        }
        out.push(String::new());
        out.push(format!("  {}", style.dim("Actions")));
        let actions = Self::actions(s);
        if actions.len() == 1 {
            out.push(format!("    {}", style.dim(&fit("No login needed (stdio, or no claude/codex config).", 4))));
        }
        for (i, a) in actions.iter().enumerate() {
            let text = match a {
                Action::Login(scope) => format!("Log in with {scope}"),
                Action::Back => "Back".to_string(),
            };
            out.push(if i == action { format!("  {} {}", style.cyan("❯"), style.bold(&text)) } else { format!("    {text}") });
        }
        out.push(String::new());
        out.push(style.dim(&truncate_to_width("  ↑↓ choose · Enter run · ←/Esc back", width)));
        out
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
    filter.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// How many servers fit on a screen of `rows` rows (about nine for the chrome), at most 15.
pub fn page_for(rows: Option<usize>) -> usize {
    rows.map_or(10, |r| r.saturating_sub(9).clamp(1, 15))
}

/// Show the picker until the person picks a login or leaves. Health answers arriving on
/// `updates` redraw it. Without a terminal, or with no servers: `None`.
#[cfg(unix)]
pub fn run(
    servers: Vec<McpServer>,
    updates: std::sync::mpsc::Receiver<crate::backend::infrastructure::mcp_health_check::Answer>,
    style: Style,
    rows: Option<usize>,
    width: usize,
) -> Option<(String, McpScope)> {
    use super::raw_keys::{Block, RawKeys, Read};
    use std::sync::mpsc::TryRecvError;

    if servers.is_empty() {
        return None;
    }
    let keys = RawKeys::enter()?;
    let mut picker = McpPicker::new(servers, page_for(rows));
    let mut block = Block::default();
    block.draw(&picker.render(width, style));
    let mut listening = true;
    let chosen = 'keys: loop {
        // Answers first, so a dot changes even while no key is pressed.
        let mut changed = false;
        while listening {
            match updates.try_recv() {
                Ok((name, scope, health)) => {
                    picker.set_health(&name, scope, health);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => listening = false,
            }
        }
        let bytes = match keys.read(Some(100)) {
            Read::Bytes(bytes) => bytes,
            Read::Timeout => Vec::new(),
            Read::Closed => break None,
        };
        for key in decode(&bytes) {
            match picker.press(key) {
                Outcome::Continue => changed = true,
                Outcome::Login { server, scope } => break 'keys Some((server, scope)),
                Outcome::Cancelled => break 'keys None,
            }
        }
        if changed {
            block.draw(&picker.render(width, style));
        }
    };
    block.clear();
    chosen
}

#[cfg(not(unix))]
pub fn run(
    _servers: Vec<McpServer>,
    _updates: std::sync::mpsc::Receiver<crate::backend::infrastructure::mcp_health_check::Answer>,
    _style: Style,
    _rows: Option<usize>,
    _width: usize,
) -> Option<(String, McpScope)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::infrastructure::mcp_discovery::{McpRow, McpTransport};
    use crate::backend::infrastructure::mcp_server::group;

    fn row(name: &str, scope: McpScope, transport: McpTransport) -> McpRow {
        McpRow { name: name.into(), scope, transport, target: format!("https://{name}.example/mcp") }
    }

    /// luna: http under claude and codex; pw: stdio under claude; posthog: http under claude.
    fn servers() -> Vec<McpServer> {
        group(&[
            row("luna", McpScope::Claude, McpTransport::Http),
            row("pw", McpScope::Claude, McpTransport::Stdio),
            row("luna", McpScope::Codex, McpTransport::Http),
            row("posthog", McpScope::Claude, McpTransport::Http),
        ])
    }

    fn text(p: &McpPicker) -> String {
        p.render(70, Style::off()).join("\n")
    }

    #[test]
    fn arrows_and_text_decode_to_keys() {
        assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
        assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
        assert_eq!(decode(b"\x1b[C"), vec![Key::Right]);
        assert_eq!(decode(b"\x1bOD"), vec![Key::Left]);
        assert_eq!(decode(b"\x1b[5~"), vec![Key::PageUp]);
        assert_eq!(decode(b"\x1b"), vec![Key::Cancel]);
        assert_eq!(decode(b"\x1b[H"), vec![]);
        assert_eq!(decode(b"\r"), vec![Key::Enter]);
        assert_eq!(decode(b"\x03"), vec![Key::Interrupt]);
        assert_eq!(decode(b"lu"), vec![Key::Char('l'), Key::Char('u')]);
    }

    #[test]
    fn one_row_per_server_with_its_providers() {
        let t = text(&McpPicker::new(servers(), 10));
        assert!(t.contains("❯ ◌ luna"), "{t}");
        assert!(t.contains("http · claude · codex"), "{t}");
        assert!(t.contains("stdio · claude"), "{t}");
        assert_eq!(t.matches("luna").count(), 1, "{t}");
        assert!(t.contains("checking servers…"), "{t}");
    }

    #[test]
    fn answers_fill_in_the_dots() {
        let mut p = McpPicker::new(servers(), 10);
        p.set_health("luna", McpScope::Claude, McpHealth::Connected);
        p.set_health("luna", McpScope::Codex, McpHealth::Unknown);
        p.set_health("pw", McpScope::Claude, McpHealth::Failed("spawn npx ENOENT".into()));
        p.set_health("posthog", McpScope::Claude, McpHealth::NeedsLogin);
        p.set_health("nobody", McpScope::Claude, McpHealth::Connected);
        let t = text(&p);
        assert!(t.contains("● luna"), "{t}");
        assert!(t.contains("✗ pw") && t.contains("failed"), "{t}");
        assert!(t.contains("◐ posthog") && t.contains("needs login"), "{t}");
        assert!(!t.contains("checking servers…"), "{t}");
    }

    #[test]
    fn the_cursor_moves_pages_and_stops_at_the_ends() {
        let rows: Vec<McpRow> = (0..6).map(|i| row(&format!("s{i}"), McpScope::Claude, McpTransport::Http)).collect();
        let mut p = McpPicker::new(group(&rows), 2);
        p.press(Key::Up);
        for _ in 0..3 {
            p.press(Key::Down);
        }
        let t = text(&p);
        assert!(t.contains("❯ ◌ s3") && !t.contains("s0") && t.contains("3-4 of 6"), "{t}");
        p.press(Key::PageDown);
        p.press(Key::PageDown);
        assert!(text(&p).contains("❯ ◌ s5"));
        p.press(Key::PageUp);
        assert!(text(&p).contains("❯ ◌ s3"));
    }

    #[test]
    fn typing_narrows_and_esc_clears_before_closing() {
        let mut p = McpPicker::new(servers(), 10);
        for c in "POST".chars() {
            p.press(Key::Char(c));
        }
        let t = text(&p);
        assert!(t.contains("❯ ◌ posthog") && !t.contains("luna"), "{t}");
        assert_eq!(p.press(Key::Cancel), Outcome::Continue);
        assert!(text(&p).contains("luna"), "Esc cleared the search");
        assert_eq!(p.press(Key::Cancel), Outcome::Cancelled);
        let mut p = McpPicker::new(servers(), 10);
        p.press(Key::Char('z'));
        assert!(text(&p).contains("No server matches."));
        assert_eq!(p.press(Key::Enter), Outcome::Continue, "nothing to open");
        p.press(Key::Backspace);
        assert!(text(&p).contains("luna"));
        assert_eq!(McpPicker::new(servers(), 10).press(Key::Interrupt), Outcome::Cancelled);
    }

    #[test]
    fn providers_and_transport_are_searchable() {
        let mut p = McpPicker::new(servers(), 10);
        for c in "codex".chars() {
            p.press(Key::Char(c));
        }
        let t = text(&p);
        assert!(t.contains("luna") && !t.contains("posthog") && !t.contains(" pw"), "{t}");
    }

    #[test]
    fn enter_opens_the_card_and_its_logins() {
        let mut p = McpPicker::new(servers(), 10);
        p.press(Key::Enter);
        let t = text(&p);
        assert!(t.starts_with("luna"), "{t}");
        assert!(t.contains("https://luna.example/mcp"), "{t}");
        assert!(t.contains("❯ Log in with claude") && t.contains("Log in with codex") && t.contains("Back"), "{t}");
        assert_eq!(p.press(Key::Enter), Outcome::Login { server: "luna".into(), scope: McpScope::Claude });
        p.press(Key::Down);
        assert_eq!(p.press(Key::Enter), Outcome::Login { server: "luna".into(), scope: McpScope::Codex });
    }

    #[test]
    fn back_left_and_esc_return_to_the_list_where_it_was() {
        let mut p = McpPicker::new(servers(), 10);
        p.press(Key::Down);
        p.press(Key::Down);
        p.press(Key::Right);
        assert!(text(&p).starts_with("posthog"), "{}", text(&p));
        p.press(Key::Left);
        assert!(text(&p).contains("❯ ◌ posthog"), "{}", text(&p));
        for c in "ost".chars() {
            p.press(Key::Char(c));
        }
        p.press(Key::Enter);
        p.press(Key::Left);
        let t = text(&p);
        assert!(t.contains("❯ ◌ posthog") && t.contains("Search: ost"), "{t}");
        p.press(Key::Enter);
        p.press(Key::Cancel);
        assert!(text(&p).contains("❯ ◌ posthog"));
        p.press(Key::Enter);
        p.press(Key::Down);
        assert_eq!(p.press(Key::Enter), Outcome::Continue, "Back");
        assert!(text(&p).contains("❯ ◌ posthog"));
        p.press(Key::Enter);
        assert_eq!(p.press(Key::Interrupt), Outcome::Cancelled);
    }

    #[test]
    fn a_stdio_server_has_no_login() {
        let mut p = McpPicker::new(servers(), 10);
        p.press(Key::Down);
        p.press(Key::Enter);
        let t = text(&p);
        assert!(t.contains("No login needed") && t.contains("❯ Back") && !t.contains("Log in"), "{t}");
    }

    #[test]
    fn the_card_says_why_a_provider_failed() {
        let mut p = McpPicker::new(servers(), 10);
        p.set_health("luna", McpScope::Codex, McpHealth::Failed("connection refused".into()));
        p.press(Key::Enter);
        let t = text(&p);
        assert!(t.contains("◌ claude  checking"), "{t}");
        assert!(t.contains("✗ codex   failed — connection refused"), "{t}");
    }

    #[test]
    fn hostile_names_cannot_drive_the_terminal_or_overflow() {
        let long = format!("go\x1b[2J{}", "x".repeat(200));
        let mut rows = vec![row(&long, McpScope::Claude, McpTransport::Http)];
        rows[0].target = format!("https://\x1b[31m{}", "y".repeat(300));
        let mut p = McpPicker::new(group(&rows), 10);
        p.set_health(&long, McpScope::Claude, McpHealth::Failed(format!("bad\x1b]0;title\x07{}", "z".repeat(300))));
        for width in [24, 40, 70] {
            let lines = p.render(width, Style::off());
            assert!(lines.iter().all(|l| !l.contains('\x1b') && display_width(l) <= width), "{width}: {lines:?}");
        }
        p.press(Key::Enter);
        for width in [24, 40, 70] {
            let lines = p.render(width, Style::off());
            assert!(lines.iter().all(|l| !l.contains('\x1b') && display_width(l) <= width), "{width}: {lines:?}");
        }
    }

    #[test]
    fn the_page_fits_the_screen() {
        assert_eq!(page_for(Some(24)), 15);
        assert_eq!(page_for(Some(12)), 3);
        assert_eq!(page_for(Some(5)), 1);
        assert_eq!(page_for(None), 10);
    }
}
