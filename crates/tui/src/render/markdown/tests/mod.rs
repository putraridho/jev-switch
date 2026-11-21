//! Tests for `answer_text`, by what they exercise.

use super::*;
use crate::render::ansi::strip_ansi;
use crate::render::test_facts::*;
use crate::render::width::display_width;
use crate::render::MIN_WIDTH;

mod code;
mod inline;
mod layout;
mod safety;
mod tables;
