//! Tests for the syntax highlighter, by what they exercise.

use super::*;
use crate::render::strip_ansi;
use crate::render::test_facts::*;

fn hl(lang: Lang, line: &str, style: Style) -> String {
    highlight_line(lang, line, &mut LineState::Normal, style)
}

mod text;
mod tokens;
