//! Colours: the terminal's depth and a colour as red, green, blue with the conversions to SGR.

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
