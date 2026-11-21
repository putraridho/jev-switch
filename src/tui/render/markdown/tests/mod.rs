//! Tests for `answer_text`, by what they exercise.

use super::*;
use crate::tui::render::ansi::strip_ansi;
use crate::tui::render::test_facts::*;
use crate::tui::render::width::display_width;
use crate::tui::render::MIN_WIDTH;

mod code;
mod inline;
mod layout;
mod safety;
mod tables;
