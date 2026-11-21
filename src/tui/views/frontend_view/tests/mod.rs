use super::*;
use crate::tui::console::front::{ApprovalAsk, FrontMsg};
use crate::tui::input::history::History;
use crate::tui::input::menu::MENU_ROWS;
use crate::tui::render::strip_ansi;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;

mod full;
mod header;
mod input;
mod live;

pub(super) const OFF: Style = Style { enabled: false, terminal: false };
pub(super) const ON: Style = Style { enabled: true, terminal: true };

pub(super) fn app() -> App {
    App::new(Path::new("/nonexistent").into(), History::default(), crate::approval::domain::mode::Mode::Edit, "auto".into(), Vec::new())
}

pub(super) fn typed(a: &mut App, text: &str) {
    for c in text.chars() {
        a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), Instant::now());
    }
}

pub(super) fn screen(a: &App, width: usize) -> String {
    render(a, width, Instant::now(), OFF).rows.join("\n")
}
