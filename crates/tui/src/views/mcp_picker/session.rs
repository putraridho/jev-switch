use super::{decode, page_for, McpPicker, Outcome};
use jev_backend::infrastructure::mcp_discovery::McpScope;
use jev_backend::infrastructure::mcp_server::McpServer;
use crate::render::Style;

/// Show the picker until the person picks a login or leaves. Health answers arriving on
/// `updates` redraw it. Without a terminal, or with no servers: `None`.
pub fn run(
    servers: Vec<McpServer>,
    updates: std::sync::mpsc::Receiver<jev_backend::infrastructure::mcp_health_check::Answer>,
    style: Style,
    rows: Option<usize>,
    width: usize,
) -> Option<(String, McpScope)> {
    use crate::views::raw_keys::{Block, RawKeys, Read};
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
