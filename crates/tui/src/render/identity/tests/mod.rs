use super::*;
use crate::render::test_facts::*;
use crate::render::width::display_width;
use super::events::{ChipFacts, Health, NativeEvent};
use super::overrides::OverrideError;

mod builtin;
mod chips;
mod color;
mod events;
mod overrides;

pub(super) const PROVIDERS: [&str; 6] = ["claude", "codex", "gemini", "grok", "deepseek", "local"];

pub(super) fn theme(style: Style, depth: ColorDepth, palette: Palette) -> ProviderTheme {
    ProviderTheme::new(style, depth, palette, ThemeOverrides::default())
}
