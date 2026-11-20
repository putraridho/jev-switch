//! Theme packs: a named palette plus overrides, and spinner packs. Quiet by default: the
//! plain default pack stays unless the user picks another with `JEV_CODE_THEME_PACK`.

use crate::tui::render::{Palette, ThemeOverrides};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pack {
    #[default]
    Default,
    /// The colour-blind safe palette.
    ColorBlind,
    /// Icy accents.
    Winter,
    /// One grey for every provider: glyph and badge tell them apart.
    Mono,
}

pub const ALL: [Pack; 4] = [Pack::Default, Pack::ColorBlind, Pack::Winter, Pack::Mono];

impl Pack {
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ColorBlind => "colorblind",
            Self::Winter => "winter",
            Self::Mono => "mono",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ALL.into_iter().find(|p| p.name() == s.trim().to_ascii_lowercase())
    }

    /// The pack the calendar suggests: winter in December to February, otherwise the
    /// default. Used only when the user asks for seasonal themes.
    pub fn seasonal(month: u32) -> Self {
        if matches!(month, 12 | 1 | 2) { Self::Winter } else { Self::Default }
    }

    /// The palette and the overrides the pack stands for.
    pub fn theme(self) -> (Palette, ThemeOverrides) {
        let text = match self {
            Self::Default => "",
            Self::ColorBlind => "",
            Self::Winter => "claude.accent=#8ec5ff;codex.accent=#a5f3fc;gemini.accent=#c4b5fd;grok.accent=#93c5fd;deepseek.accent=#67e8f9",
            Self::Mono => "claude.accent=#999999;codex.accent=#999999;gemini.accent=#999999;grok.accent=#999999;deepseek.accent=#999999;local.accent=#999999",
        };
        let palette = if self == Self::ColorBlind { Palette::ColorBlind } else { Palette::Standard };
        (palette, ThemeOverrides::parse(text).0)
    }

    /// The working spinner's frames: the braille default, or a smaller set.
    pub fn spinner_frames(self) -> &'static [&'static str] {
        match self {
            Self::Winter => &["❄", "❅", "❆", "❅"],
            Self::Mono => &["|", "/", "-", "\\"],
            Self::Default | Self::ColorBlind => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::{ColorDepth, ProviderTheme, Style};

    #[test]
    fn names_round_trip() {
        for p in ALL {
            assert_eq!(Pack::parse(p.name()), Some(p));
        }
        assert_eq!(Pack::parse(" Winter "), Some(Pack::Winter));
        assert_eq!(Pack::parse("neon"), None);
    }

    #[test]
    fn seasonal_is_winter_only_in_winter() {
        assert_eq!(Pack::seasonal(12), Pack::Winter);
        assert_eq!(Pack::seasonal(2), Pack::Winter);
        assert_eq!(Pack::seasonal(7), Pack::Default);
    }

    #[test]
    fn packs_change_the_accent_not_the_glyph() {
        let chip = |p: Pack| {
            let (palette, overrides) = p.theme();
            ProviderTheme::new(Style::on(), ColorDepth::TrueColor, palette, overrides).chip("codex")
        };
        assert_eq!(chip(Pack::Default), "\x1b[38;2;0;175;95m▲ codex\x1b[0m");
        assert_eq!(chip(Pack::ColorBlind), "\x1b[38;2;86;180;233m▲ codex\x1b[0m");
        assert_eq!(chip(Pack::Winter), "\x1b[38;2;165;243;252m▲ codex\x1b[0m");
        assert_eq!(chip(Pack::Mono), "\x1b[38;2;153;153;153m▲ codex\x1b[0m");
    }

    #[test]
    fn every_pack_parses_cleanly_and_has_spinner_frames() {
        for p in ALL {
            assert!(p.spinner_frames().len() >= 4);
        }
        let text = "claude.accent=#8ec5ff;codex.accent=#a5f3fc;gemini.accent=#c4b5fd;grok.accent=#93c5fd;deepseek.accent=#67e8f9";
        assert!(ThemeOverrides::parse(text).1.is_empty());
    }
}
