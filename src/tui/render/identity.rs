//! Per-provider visual identity: each provider has an accent colour, a glyph and a short
//! badge, used the same way in the route row, the answer gutter, the footer chips and the
//! provider-specific event rows. Colour is never the only cue: the glyph and the badge
//! carry the same information in a monochrome terminal.
//!
//! Colour depth (truecolor, 256, 16) and the palette (standard, colour-blind safe) are
//! explicit inputs so every combination is a golden test. Users override accent, glyph or
//! badge per provider with [`ThemeOverrides`].


use crate::tui::render::style::Style;

pub mod color;
pub mod events;
pub mod overrides;
#[cfg(test)]
mod tests;

use color::{ColorDepth, Rgb};
use overrides::ThemeOverrides;

/// Which set of accents to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Palette {
    #[default]
    Standard,
    /// Okabe-Ito colours, distinguishable with the common colour-vision deficiencies.
    ColorBlind,
}

impl Palette {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "standard" | "default" => Some(Self::Standard),
            "colorblind" | "colourblind" | "color-blind" | "colour-blind" => Some(Self::ColorBlind),
            _ => None,
        }
    }
}

/// The look of one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub accent: Rgb,
    pub glyph: char,
    pub badge: String,
}

/// The built-in identity of `provider`; an unknown provider is a neutral grey dot.
pub fn builtin_identity(provider: &str, palette: Palette) -> Identity {
    let name = provider.to_ascii_lowercase();
    let (standard, blind, glyph, badge) = match name.as_str() {
        "claude" => (Rgb(255, 135, 0), Rgb(230, 159, 0), '◆', "claude"),
        "codex" => (Rgb(0, 175, 95), Rgb(86, 180, 233), '▲', "codex"),
        "gemini" => (Rgb(175, 95, 215), Rgb(204, 121, 167), '✦', "gemini"),
        "grok" => (Rgb(95, 135, 255), Rgb(60, 150, 210), '✕', "grok"),
        "deepseek" => (Rgb(0, 175, 215), Rgb(0, 158, 115), '◉', "deepseek"),
        "local" => (Rgb(150, 150, 150), Rgb(150, 150, 150), '○', "local"),
        _ => (Rgb(150, 150, 150), Rgb(150, 150, 150), '•', ""),
    };
    let badge = if badge.is_empty() { name.chars().take(8).collect() } else { badge.to_string() };
    Identity { accent: if palette == Palette::Standard { standard } else { blind }, glyph, badge }
}

/// The identity of providers as one terminal shows them: palette, colour depth, overrides.
#[derive(Debug, Clone)]
pub struct ProviderTheme {
    style: Style,
    depth: ColorDepth,
    palette: Palette,
    overrides: ThemeOverrides,
}

impl ProviderTheme {
    pub fn new(style: Style, depth: ColorDepth, palette: Palette, overrides: ThemeOverrides) -> Self {
        Self { style, depth, palette, overrides }
    }

    /// From the environment: `JEV_CODE_PALETTE=colorblind`, `JEV_CODE_THEME_OVERRIDES=
    /// claude.accent=#d97706;codex.glyph=*`, and the terminal's colour depth. Unknown values
    /// fall back to the defaults.
    pub fn from_env(style: Style) -> Self {
        let palette = std::env::var("JEV_CODE_PALETTE").ok().and_then(|p| Palette::parse(&p)).unwrap_or_default();
        let (overrides, _) = ThemeOverrides::parse(&std::env::var("JEV_CODE_THEME_OVERRIDES").unwrap_or_default());
        Self::new(style, ColorDepth::detect(), palette, overrides)
    }

    pub fn identity(&self, provider: &str) -> Identity {
        self.overrides.apply(provider, builtin_identity(provider, self.palette))
    }

    /// `text` in the accent of `provider`.
    pub fn paint(&self, provider: &str, text: &str) -> String {
        self.style.paint(&self.identity(provider).accent.sgr(self.depth), text)
    }

    /// `◆ claude`: glyph and badge in the accent.
    pub fn chip(&self, provider: &str) -> String {
        let id = self.identity(provider);
        self.paint(provider, &format!("{} {}", id.glyph, id.badge))
    }
}
