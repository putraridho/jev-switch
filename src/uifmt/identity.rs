//! Per-provider visual identity: each provider has an accent colour, a glyph and a short
//! badge, used the same way in the route row, the answer gutter, the footer chips and the
//! provider-specific event rows. Colour is never the only cue: the glyph and the badge
//! carry the same information in a monochrome terminal.
//!
//! Colour depth (truecolor, 256, 16) and the palette (standard, colour-blind safe) are
//! explicit inputs so every combination is a golden test. Users override accent, glyph or
//! badge per provider with [`ThemeOverrides`].

use std::collections::BTreeMap;

use super::style::Style;
use super::width::{clamp_width, display_width, truncate_to_width};
use super::wrap::wrap_to_width;
use crate::gimmicks::gauge::{fuel_gauge, FuelLevel};

/// How many colours the terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
}

impl ColorDepth {
    /// From `COLORTERM` (`truecolor`/`24bit`) and `TERM` (`*256color`); anything else is 16.
    pub fn resolve(colorterm: Option<&str>, term: Option<&str>) -> Self {
        if colorterm.is_some_and(|c| matches!(c.to_ascii_lowercase().as_str(), "truecolor" | "24bit")) {
            Self::TrueColor
        } else if term.is_some_and(|t| t.contains("256color") || t.contains("direct")) {
            Self::Ansi256
        } else {
            Self::Ansi16
        }
    }

    pub fn detect() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self::resolve(var("COLORTERM").as_deref(), var("TERM").as_deref())
    }
}

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

/// A colour as red, green, blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// The sixteen base colours as most terminals draw them, in SGR order (30-37, then 90-97).
const ANSI16: [Rgb; 16] = [
    Rgb(0, 0, 0),
    Rgb(205, 0, 0),
    Rgb(0, 205, 0),
    Rgb(205, 205, 0),
    Rgb(0, 0, 238),
    Rgb(205, 0, 205),
    Rgb(0, 205, 205),
    Rgb(229, 229, 229),
    Rgb(127, 127, 127),
    Rgb(255, 0, 0),
    Rgb(0, 255, 0),
    Rgb(255, 255, 0),
    Rgb(92, 92, 255),
    Rgb(255, 0, 255),
    Rgb(0, 255, 255),
    Rgb(255, 255, 255),
];

impl Rgb {
    /// `#rrggbb` (the `#` is optional).
    pub fn parse_hex(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 || !h.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Self(byte(0)?, byte(2)?, byte(4)?))
    }

    fn distance(self, o: Self) -> u32 {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2) as u32;
        d(self.0, o.0) + d(self.1, o.1) + d(self.2, o.2)
    }

    /// WCAG relative luminance, 0 (black) to 1 (white).
    pub fn luminance(self) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }

    /// WCAG contrast ratio against `other`.
    pub fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn to_256(self) -> u8 {
        // The 6x6x6 cube's levels are not evenly spaced: 0, 95, 135, 175, 215, 255.
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let level = |c: u8| LEVELS.iter().enumerate().min_by_key(|(_, l)| i32::from(c).abs_diff(i32::from(**l))).map_or(0, |(i, _)| i as u32);
        (16 + 36 * level(self.0) + 6 * level(self.1) + level(self.2)) as u8
    }

    fn to_16(self) -> usize {
        ANSI16.iter().enumerate().min_by_key(|(_, c)| self.distance(**c)).map_or(7, |(i, _)| i)
    }

    /// The SGR parameters selecting this colour as a foreground at `depth`.
    pub fn sgr(self, depth: ColorDepth) -> String {
        match depth {
            ColorDepth::TrueColor => format!("38;2;{};{};{}", self.0, self.1, self.2),
            ColorDepth::Ansi256 => format!("38;5;{}", self.to_256()),
            ColorDepth::Ansi16 => {
                let i = self.to_16();
                (if i < 8 { 30 + i } else { 90 + i - 8 }).to_string()
            }
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

/// What a user changed about one provider's look; anything left `None` keeps the built-in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityPatch {
    pub accent: Option<Rgb>,
    pub glyph: Option<char>,
    pub badge: Option<String>,
}

/// Why one override line was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideError {
    /// Not `provider.field=value`.
    Malformed(String),
    UnknownField(String),
    BadColour(String),
    /// A glyph must be one character that takes one column.
    BadGlyph(String),
    /// A badge is 1 to 8 columns.
    BadBadge(String),
}

impl std::fmt::Display for OverrideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(l) => write!(f, "`{l}` is not provider.field=value"),
            Self::UnknownField(x) => write!(f, "unknown field `{x}` (accent, glyph, badge)"),
            Self::BadColour(v) => write!(f, "`{v}` is not a #rrggbb colour"),
            Self::BadGlyph(v) => write!(f, "`{v}` is not a single one-column character"),
            Self::BadBadge(v) => write!(f, "`{v}` is not a badge of 1 to 8 columns"),
        }
    }
}

/// Per-provider overrides: `claude.accent=#d97706`, `codex.glyph=*`, `gemini.badge=agy`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    patches: BTreeMap<String, IdentityPatch>,
}

impl ThemeOverrides {
    /// Parse override lines separated by newlines or `;`. Blank lines and `#` comment lines
    /// are skipped; a bad line is reported and the others still apply.
    pub fn parse(text: &str) -> (Self, Vec<OverrideError>) {
        let mut out = Self::default();
        let mut errors = Vec::new();
        for line in text.split(['\n', ';']).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Err(e) = out.apply_line(line) {
                errors.push(e);
            }
        }
        (out, errors)
    }

    fn apply_line(&mut self, line: &str) -> Result<(), OverrideError> {
        let malformed = || OverrideError::Malformed(line.to_string());
        let (key, value) = line.split_once('=').ok_or_else(malformed)?;
        let (provider, field) = key.trim().split_once('.').ok_or_else(malformed)?;
        let value = value.trim();
        let patch = self.patches.entry(provider.trim().to_ascii_lowercase()).or_default();
        match field.trim() {
            "accent" => patch.accent = Some(Rgb::parse_hex(value).ok_or_else(|| OverrideError::BadColour(value.into()))?),
            "glyph" => {
                let mut cs = value.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) if display_width(&c.to_string()) == 1 => patch.glyph = Some(c),
                    _ => return Err(OverrideError::BadGlyph(value.into())),
                }
            }
            "badge" => {
                let w = display_width(value);
                if w == 0 || w > 8 || value.chars().any(char::is_control) {
                    return Err(OverrideError::BadBadge(value.into()));
                }
                patch.badge = Some(value.to_string());
            }
            other => return Err(OverrideError::UnknownField(other.into())),
        }
        Ok(())
    }

    /// Layer `other` over `self` (what `other` sets wins).
    pub fn merged(mut self, other: &Self) -> Self {
        for (provider, p) in &other.patches {
            let mine = self.patches.entry(provider.clone()).or_default();
            mine.accent = p.accent.or(mine.accent);
            mine.glyph = p.glyph.or(mine.glyph);
            mine.badge = p.badge.clone().or_else(|| mine.badge.take());
        }
        self
    }

    fn apply(&self, provider: &str, mut base: Identity) -> Identity {
        if let Some(p) = self.patches.get(&provider.to_ascii_lowercase()) {
            base.accent = p.accent.unwrap_or(base.accent);
            base.glyph = p.glyph.unwrap_or(base.glyph);
            if let Some(b) = &p.badge {
                base.badge = b.clone();
            }
        }
        base
    }
}

/// Health of a provider as the footer chip shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    CoolingDown,
    Unavailable,
}

/// What a footer chip says about one provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ChipFacts {
    pub plan: Option<String>,
    /// Fraction of the plan window used, 0..1.
    pub used: Option<f64>,
    pub cooldown: Option<String>,
    pub health: Health,
}

/// A native event of a backend, shown faithfully in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeEvent<'a> {
    Thinking(&'a str),
    Patch { files: &'a [String] },
    PlanStep { index: usize, total: usize, text: &'a str },
}

/// The lines of thinking or patch files shown before the rest is counted.
fn collapse_limit(provider: &str, event: &NativeEvent<'_>) -> usize {
    match (provider, event) {
        ("claude", NativeEvent::Thinking(_)) => 4,
        (_, NativeEvent::Thinking(_)) => 2,
        (_, NativeEvent::Patch { .. }) => 5,
        (_, NativeEvent::PlanStep { .. }) => 1,
    }
}

/// How a provider names its own events.
fn event_label(provider: &str, event: &NativeEvent<'_>) -> &'static str {
    match (provider, event) {
        ("codex", NativeEvent::Thinking(_)) => "reasoning",
        ("gemini", NativeEvent::Thinking(_)) => "thoughts",
        (_, NativeEvent::Thinking(_)) => "thinking",
        ("codex", NativeEvent::Patch { .. }) => "apply_patch",
        (_, NativeEvent::Patch { .. }) => "patch",
        (_, NativeEvent::PlanStep { .. }) => "plan",
    }
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

    /// The header above an answer: who wrote it and with which model. A failover or race
    /// names every provider involved, in order.
    pub fn attribution_header(&self, authors: &[(&str, &str)]) -> String {
        let parts: Vec<String> = authors
            .iter()
            .map(|(p, m)| if m.is_empty() { self.chip(p) } else { format!("{} {} {m}", self.chip(p), self.style.dim("·")) })
            .collect();
        parts.join(&self.style.dim("  ⇄  "))
    }

    /// The answer set apart with a header and a thin gutter in the author's colour.
    /// `laid_out` must already fit `width - 2` columns.
    pub fn attributed_answer(&self, authors: &[(&str, &str)], laid_out: &str) -> String {
        let Some((first, _)) = authors.first() else { return format!("\n{}", laid_out.trim_end_matches('\n')) };
        let bar = self.paint(first, "▌");
        let mut rows = vec![String::new(), format!("{bar} {}", self.attribution_header(authors))];
        rows.extend(laid_out.trim_end_matches('\n').split('\n').map(|l| format!("{bar} {l}")));
        rows.join("\n")
    }

    /// A footer chip: `◆ claude max_5x ▰▰▰▱▱ 60% ⏳ 2h 10m`.
    pub fn footer_chip(&self, provider: &str, f: &ChipFacts) -> String {
        let mut parts = vec![self.chip(provider)];
        if let Some(plan) = &f.plan {
            parts.push(self.style.dim(plan));
        }
        if let Some(used) = f.used {
            let g = fuel_gauge(used, 5);
            let text = format!("{} {}%", g.bar, g.percent);
            parts.push(match g.level {
                FuelLevel::Ok => self.style.dim(&text),
                FuelLevel::Low => self.style.yellow(&text),
                FuelLevel::Critical => self.style.red(&text),
            });
        }
        match (f.health, &f.cooldown) {
            (Health::CoolingDown, Some(t)) => parts.push(self.style.yellow(&format!("⏳ {t}"))),
            (Health::CoolingDown, None) => parts.push(self.style.yellow("⏳")),
            (Health::Unavailable, _) => parts.push(self.style.red("✗ unavailable")),
            (Health::Ok, _) => {}
        }
        parts.join(" ")
    }

    /// A backend's own event under its own label, collapsed the way that backend's events
    /// are read (Claude's thinking longest, Codex patches as a file list, Gemini's plan
    /// steps numbered).
    pub fn native_event(&self, provider: &str, event: &NativeEvent<'_>, width: usize) -> String {
        let width = clamp_width(width);
        let label = event_label(provider, event);
        let head = format!("  {} {}", self.paint(provider, &self.identity(provider).glyph.to_string()), self.style.dim(label));
        let limit = collapse_limit(provider, event);
        let (detail, total): (Vec<String>, usize) = match event {
            NativeEvent::Thinking(text) => {
                let lines: Vec<String> = text.trim().lines().flat_map(|l| wrap_to_width(l, width.saturating_sub(6))).collect();
                let n = lines.len();
                (lines.into_iter().take(limit).map(|l| self.style.dim(&self.style.italic(&l))).collect(), n)
            }
            NativeEvent::Patch { files } => (files.iter().take(limit).map(|f| format!("{} {f}", self.style.green("+"))).collect(), files.len()),
            NativeEvent::PlanStep { index, total, text } => return truncate_to_width(&format!("{head} {}", self.style.dim(&format!("{index}/{total} {text}"))), width),
        };
        let mut rows = vec![head];
        rows.extend(detail.into_iter().map(|d| truncate_to_width(&format!("    {d}"), width)));
        if total > limit {
            rows.push(format!("    {}", self.style.dim(&format!("… {} more", total - limit))));
        }
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::strip_ansi;
    use crate::uifmt::testutil::*;

    const PROVIDERS: [&str; 6] = ["claude", "codex", "gemini", "grok", "deepseek", "local"];

    fn theme(style: Style, depth: ColorDepth, palette: Palette) -> ProviderTheme {
        ProviderTheme::new(style, depth, palette, ThemeOverrides::default())
    }

    #[test]
    fn depth_detection_table() {
        for (ct, term, want) in [
            (Some("truecolor"), Some("xterm"), ColorDepth::TrueColor),
            (Some("24bit"), None, ColorDepth::TrueColor),
            (None, Some("xterm-256color"), ColorDepth::Ansi256),
            (None, Some("screen"), ColorDepth::Ansi16),
            (None, None, ColorDepth::Ansi16),
        ] {
            assert_eq!(ColorDepth::resolve(ct, term), want);
        }
    }

    #[test]
    fn colour_sequences_per_depth() {
        let orange = Rgb(255, 135, 0);
        assert_eq!(orange.sgr(ColorDepth::TrueColor), "38;2;255;135;0");
        assert_eq!(orange.sgr(ColorDepth::Ansi256), "38;5;208");
        assert_eq!(Rgb(0, 205, 0).sgr(ColorDepth::Ansi16), "32");
        assert_eq!(Rgb(255, 0, 0).sgr(ColorDepth::Ansi16), "91");
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(Rgb::parse_hex("#ff8700"), Some(Rgb(255, 135, 0)));
        assert_eq!(Rgb::parse_hex("00ff00"), Some(Rgb(0, 255, 0)));
        for bad in ["", "#fff", "#gggggg", "#ff87000", "ééé"] {
            assert_eq!(Rgb::parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_accent_is_readable_on_dark_and_light_terminals() {
        for palette in [Palette::Standard, Palette::ColorBlind] {
            for p in PROVIDERS {
                let a = builtin_identity(p, palette).accent;
                assert!(a.contrast(Rgb(0, 0, 0)) >= 4.5, "{p} on black");
                assert!(a.contrast(Rgb(255, 255, 255)) >= 2.0, "{p} on white");
            }
        }
    }

    #[test]
    fn providers_differ_by_glyph_and_badge_not_only_colour() {
        let ids: Vec<Identity> = PROVIDERS.iter().map(|p| builtin_identity(p, Palette::Standard)).collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a.glyph, b.glyph);
                assert_ne!(a.badge, b.badge);
            }
        }
        for palette in [Palette::Standard, Palette::ColorBlind] {
            let colours: Vec<Rgb> = PROVIDERS[..5].iter().map(|p| builtin_identity(p, palette).accent).collect();
            for (i, a) in colours.iter().enumerate() {
                assert!(colours[i + 1..].iter().all(|b| a != b));
            }
        }
        assert_eq!(builtin_identity("mystery-provider", Palette::Standard).badge, "mystery-");
    }

    #[test]
    fn golden_chip_per_provider_and_depth() {
        let golden = [
            ("claude", ColorDepth::TrueColor, "\x1b[38;2;255;135;0m◆ claude\x1b[0m"),
            ("claude", ColorDepth::Ansi256, "\x1b[38;5;208m◆ claude\x1b[0m"),
            ("codex", ColorDepth::Ansi16, "\x1b[32m▲ codex\x1b[0m"),
            ("gemini", ColorDepth::Ansi256, "\x1b[38;5;134m✦ gemini\x1b[0m"),
            ("grok", ColorDepth::TrueColor, "\x1b[38;2;95;135;255m✕ grok\x1b[0m"),
            ("deepseek", ColorDepth::Ansi16, "\x1b[36m◉ deepseek\x1b[0m"),
        ];
        for (p, depth, want) in golden {
            assert_eq!(theme(ON, depth, Palette::Standard).chip(p), want, "{p} {depth:?}");
        }
        assert_eq!(theme(ON, ColorDepth::TrueColor, Palette::ColorBlind).chip("codex"), "\x1b[38;2;86;180;233m▲ codex\x1b[0m");
    }

    #[test]
    fn a_disabled_style_is_plain_text_with_glyph_and_badge() {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            let th = theme(OFF, depth, Palette::Standard);
            assert_eq!(th.chip("codex"), "▲ codex");
            no_esc(&th.attribution_header(&[("claude", "m"), ("codex", "n")]));
            no_esc(&th.footer_chip("claude", &ChipFacts { plan: Some("max".into()), used: Some(0.9), cooldown: None, health: Health::Ok }));
        }
    }

    #[test]
    fn the_answer_gets_a_gutter_and_a_header_naming_the_author() {
        let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
        assert_eq!(th.attributed_answer(&[("codex", "gpt-6.1-sol")], "one\ntwo\n"), "\n▌ ▲ codex · gpt-6.1-sol\n▌ one\n▌ two");
        assert_eq!(
            th.attributed_answer(&[("claude", "opus"), ("codex", "")], "x"),
            "\n▌ ◆ claude · opus  ⇄  ▲ codex\n▌ x"
        );
        assert_eq!(th.attributed_answer(&[], "x\n"), "\nx");
        let coloured = theme(ON, ColorDepth::Ansi16, Palette::Standard).attributed_answer(&[("codex", "m")], "one");
        assert_eq!(strip_ansi(&coloured), "\n▌ ▲ codex · m\n▌ one");
        assert!(coloured.contains("\x1b[32m▌"));
    }

    #[test]
    fn footer_chip_shows_plan_gauge_and_cooldown() {
        let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
        let facts = ChipFacts { plan: Some("max_5x".into()), used: Some(0.6), cooldown: None, health: Health::Ok };
        assert_eq!(th.footer_chip("claude", &facts), "◆ claude max_5x ▰▰▰▱▱ 60%");
        let hot = ChipFacts { used: Some(0.97), cooldown: Some("2h 10m".into()), health: Health::CoolingDown, ..facts.clone() };
        assert_eq!(th.footer_chip("claude", &hot), "◆ claude max_5x ▰▰▰▰▰ 97% ⏳ 2h 10m");
        let down = ChipFacts { plan: None, used: None, cooldown: None, health: Health::Unavailable };
        assert_eq!(th.footer_chip("gemini", &down), "✦ gemini ✗ unavailable");
        let styled = theme(ON, ColorDepth::Ansi16, Palette::Standard).footer_chip("claude", &hot);
        assert!(styled.contains("\x1b[31m▰▰▰▰▰ 97%"), "critical is red: {styled:?}");
    }

    #[test]
    fn native_events_use_each_providers_words_and_collapse() {
        let th = theme(OFF, ColorDepth::Ansi16, Palette::Standard);
        let long = "a\nb\nc\nd\ne\nf";
        assert_eq!(th.native_event("claude", &NativeEvent::Thinking(long), 80), "  ◆ thinking\n    a\n    b\n    c\n    d\n    … 2 more");
        assert_eq!(th.native_event("codex", &NativeEvent::Thinking(long), 80), "  ▲ reasoning\n    a\n    b\n    … 4 more");
        assert_eq!(th.native_event("gemini", &NativeEvent::Thinking("x"), 80), "  ✦ thoughts\n    x");
        let files = vec!["a.rs".to_string(), "b.rs".to_string()];
        assert_eq!(th.native_event("codex", &NativeEvent::Patch { files: &files }, 80), "  ▲ apply_patch\n    + a.rs\n    + b.rs");
        assert_eq!(th.native_event("gemini", &NativeEvent::PlanStep { index: 2, total: 5, text: "write tests" }, 80), "  ✦ plan 2/5 write tests");
        for row in th.native_event("claude", &NativeEvent::Thinking(&"word ".repeat(40)), 30).lines() {
            assert!(display_width(row) < 30, "{row:?}");
        }
    }

    #[test]
    fn overrides_change_accent_glyph_and_badge() {
        let (o, errors) = ThemeOverrides::parse("claude.accent=#112233; codex.glyph=*\n# comment\ngemini.badge=agy");
        assert!(errors.is_empty(), "{errors:?}");
        let th = ProviderTheme::new(ON, ColorDepth::TrueColor, Palette::Standard, o);
        assert_eq!(th.chip("claude"), "\x1b[38;2;17;34;51m◆ claude\x1b[0m");
        assert_eq!(strip_ansi(&th.chip("codex")), "* codex");
        assert_eq!(strip_ansi(&th.chip("gemini")), "✦ agy");
    }

    #[test]
    fn bad_override_lines_are_reported_and_the_rest_apply() {
        let (o, errors) = ThemeOverrides::parse("claude.accent=red;codex.glyph=ab;grok.badge=;nothing;a.b=c;gemini.glyph=✦;deepseek.badge=ds");
        assert_eq!(errors.len(), 5);
        assert!(matches!(errors[0], OverrideError::BadColour(_)));
        assert!(matches!(errors[1], OverrideError::BadGlyph(_)));
        assert!(matches!(errors[2], OverrideError::BadBadge(_)));
        assert!(matches!(errors[3], OverrideError::Malformed(_)));
        assert!(matches!(errors[4], OverrideError::UnknownField(_)));
        assert!(errors.iter().all(|e| !e.to_string().is_empty()));
        let th = ProviderTheme::new(OFF, ColorDepth::Ansi16, Palette::Standard, o);
        assert_eq!(th.chip("deepseek"), "◉ ds");
        assert_eq!(th.chip("claude"), "◆ claude");
    }

    #[test]
    fn merged_overrides_prefer_the_later_layer() {
        let (a, _) = ThemeOverrides::parse("claude.badge=one;claude.glyph=+");
        let (b, _) = ThemeOverrides::parse("claude.badge=two");
        let th = ProviderTheme::new(OFF, ColorDepth::Ansi16, Palette::Standard, a.merged(&b));
        assert_eq!(th.chip("claude"), "+ two");
    }

    #[test]
    fn palette_names() {
        assert_eq!(Palette::parse("Colour-Blind"), Some(Palette::ColorBlind));
        assert_eq!(Palette::parse("default"), Some(Palette::Standard));
        assert_eq!(Palette::parse("neon"), None);
    }
}
