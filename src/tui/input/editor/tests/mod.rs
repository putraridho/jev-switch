//! Tests for the text editor, by what they exercise.

use super::*;
use crate::tui::render::display_width;

fn ed(text: &str, cursor: usize) -> Editor {
    Editor { text: text.into(), cursor }
}

mod edit;
mod layout;
mod motion;
