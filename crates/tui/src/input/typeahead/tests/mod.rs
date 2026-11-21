//! Tests for type-ahead, by what they exercise.

use super::draft::{Draft, Effect};
use super::keys::{keys, Key};
use super::reader::handle_chunk;
use crate::console::terminal_ui::Console;

fn text(s: &str) -> Key {
    Key::Text(s.into())
}

mod console;
mod draft;
mod keys;
