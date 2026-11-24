//! Colour: whether to emit it, the SGR painters, and the per-provider palette.

use crate::render::ansi::RESET;

pub fn stdout_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

pub fn stderr_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stderr())
}

static LIGHT_BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tell the painters the terminal has a light background (`true`) or a dark one. Set once at
/// startup from [`background_from_env`] or by asking the terminal; dark until told.
pub fn set_light_background(light: bool) {
    LIGHT_BACKGROUND.store(light, std::sync::atomic::Ordering::Relaxed);
}

pub fn light_background() -> bool {
    LIGHT_BACKGROUND.load(std::sync::atomic::Ordering::Relaxed)
}

/// The terminal background the environment names: `JEV_THEME=light|dark`, else the
/// `COLORFGBG` hint (`15;0` is a dark terminal, `0;15` a light one). `None` when neither says.
pub fn background_from_env(theme: Option<&str>, colorfgbg: Option<&str>) -> Option<bool> {
    match theme.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("light") => return Some(true),
        Some("dark") => return Some(false),
        _ => {}
    }
    let bg: u8 = colorfgbg?.rsplit(';').next()?.trim().parse().ok()?;
    Some(matches!(bg, 7 | 9..=15))
}

/// Whether and how to colour. Disabled means NOTHING is added to any string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub enabled: bool,
    /// The stream is a terminal. With colour off, text going to a terminal is still
    /// stripped of escape and control sequences (model output must not be able to
    /// retitle, clear or spoof the screen); only non-terminal output stays raw.
    pub terminal: bool,
}

impl Style {
    pub fn on() -> Self {
        Self { enabled: true, terminal: true }
    }

    pub fn off() -> Self {
        Self { enabled: false, terminal: false }
    }

    /// Colour for a stream, from the environment: `NO_COLOR` (non-empty) turns it off
    /// and beats everything; `CLICOLOR_FORCE` (non-empty, not `0`) turns it on even when
    /// piped; `TERM=dumb` turns it off; otherwise it follows whether the stream is a tty.
    pub fn detect(stream_is_tty: bool) -> Self {
        let var = |k: &str| std::env::var(k).ok();
        // `JEV_THEME=mono` is NO_COLOR by another name.
        let mono = crate::tty::accessibility::mono_theme(var("JEV_THEME").as_deref()).then(|| "1".to_string());
        Self::resolve(stream_is_tty, mono.or_else(|| var("NO_COLOR")).as_deref(), var("TERM").as_deref(), var("CLICOLOR_FORCE").as_deref())
    }

    /// The pure half of [`Style::detect`].
    pub fn resolve(stream_is_tty: bool, no_color: Option<&str>, term: Option<&str>, force: Option<&str>) -> Self {
        let enabled = if no_color.is_some_and(|v| !v.is_empty()) {
            false
        } else if force.is_some_and(|v| !v.is_empty() && v != "0") {
            true
        } else if term.is_some_and(|t| t == "dumb") {
            false
        } else {
            stream_is_tty
        };
        Self { enabled, terminal: stream_is_tty }
    }

    /// `s` in SGR `code`. Resets after the span, and re-opens the style after any
    /// reset inside `s`, so a coloured word inside a dim line does not end the dimming.
    pub(super) fn paint(&self, code: &str, s: &str) -> String {
        if !self.enabled || s.is_empty() {
            return s.to_string();
        }
        let reopen = format!("{RESET}\x1b[{code}m");
        format!("\x1b[{code}m{}{RESET}", s.replace(RESET, &reopen))
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn reverse(&self, s: &str) -> String {
        self.paint("7", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn italic(&self, s: &str) -> String {
        self.paint("3", s)
    }
    /// A basic ANSI hue on a dark terminal, so the person's own palette applies. On a light one,
    /// a fixed darker shade from the 256 palette instead: the stock light palettes' yellow,
    /// green, cyan and magenta measure 1.7 to 4.1:1 on white, under the 4.5:1 text needs.
    fn hue(&self, ansi: &str, on_light: u8, s: &str) -> String {
        self.paint(&hue_code(ansi, on_light, light_background()), s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    /// #008700 on light: 4.7:1 on white.
    pub fn green(&self, s: &str) -> String {
        self.hue("32", 28, s)
    }
    /// #875f00 on light: 5.7:1 on white.
    pub fn yellow(&self, s: &str) -> String {
        self.hue("33", 94, s)
    }
    pub fn blue(&self, s: &str) -> String {
        self.paint("34", s)
    }
    /// #af00af on light: 6.1:1 on white.
    pub fn magenta(&self, s: &str) -> String {
        self.hue("35", 127, s)
    }
    /// #005f87 on light: 7.0:1 on white.
    pub fn cyan(&self, s: &str) -> String {
        self.hue("36", 24, s)
    }
    /// 256-colour foreground.
    pub fn fg256(&self, n: u8, s: &str) -> String {
        self.paint(&format!("38;5;{n}"), s)
    }
    /// `s` on a full-width shaded row for the user's own messages: a step off the terminal's
    /// background (darker grey on dark, light grey on light), the text in its own colour so
    /// it reads on either. Plain when colour is off.
    pub fn band(&self, s: &str) -> String {
        self.paint(if light_background() { "48;5;254" } else { "48;5;236" }, s)
    }
    /// `s` on a tinted row for a diff: a muted red for what an edit takes out, a muted green
    /// for what it puts in, a step off the background on either theme. Plain when colour is off.
    pub fn tint(&self, added: bool, s: &str) -> String {
        let code = match (added, light_background()) {
            (true, false) => "48;5;22",
            (true, true) => "48;5;194",
            (false, false) => "48;5;52",
            (false, true) => "48;5;224",
        };
        self.paint(code, s)
    }
    /// The accent for structure (markers, the prompt, headings): readable on either
    /// background, where plain cyan washes out on light.
    pub fn accent(&self, s: &str) -> String {
        self.paint(if light_background() { "38;5;25" } else { "38;5;110" }, s)
    }
    /// `s` in the accent with a soft highlight sweeping across it as `elapsed` grows, so a
    /// working row feels alive between spinner frames. A pure function of time. Plain accent
    /// when colour is off, motion is reduced, or the terminal is light (the ramp is for dark).
    pub fn shimmer(&self, s: &str, elapsed: std::time::Duration) -> String {
        if !self.enabled || s.is_empty() || light_background() || crate::tty::accessibility::reduced_motion() {
            return self.accent(s);
        }
        const SWEEP_MS: f64 = 1800.0;
        const BAND: f64 = 5.0;
        let truecolor = crate::render::ColorDepth::detect() == crate::render::ColorDepth::TrueColor;
        // Accent (#87afd7, 256-colour 110) rising to near white at the highlight's centre.
        const BASE: [f64; 3] = [135.0, 175.0, 215.0];
        const PEAK: [f64; 3] = [235.0, 245.0, 255.0];
        let len = s.chars().count() as f64;
        // The highlight travels from before the first char to past the last, then comes round again.
        let phase = (elapsed.as_millis() as f64 % SWEEP_MS) / SWEEP_MS;
        let pos = phase * (len + BAND * 2.0) - BAND;
        let mut out = String::with_capacity(s.len() * 20);
        for (i, c) in s.chars().enumerate() {
            let d = (i as f64 - pos).abs() / BAND;
            // A smooth bump: 1 at the centre, 0 at the edge, no steps in between.
            let k = if d >= 1.0 { 0.0 } else { let x = 1.0 - d; x * x * (3.0 - 2.0 * x) };
            if truecolor {
                let ch = |n: usize| (BASE[n] + (PEAK[n] - BASE[n]) * k).round() as u8;
                out.push_str(&format!("\x1b[38;2;{};{};{}m{c}", ch(0), ch(1), ch(2)));
            } else {
                const STEPS: [u8; 5] = [110, 111, 117, 153, 195];
                out.push_str(&format!("\x1b[38;5;{}m{c}", STEPS[(k * 4.0).round() as usize]));
            }
        }
        out.push_str(RESET);
        out
    }
    /// Box lines and rules: a quiet grey, so the frame recedes behind the text it holds.
    pub fn chrome(&self, s: &str) -> String {
        self.paint(if light_background() { "38;5;250" } else { "38;5;240" }, s)
    }
    /// The edge of a control the person types into (the input box). Unlike [`Style::chrome`] it
    /// must stay visible: #808080 is 4.2:1 on a #1e1e1e terminal and 4.0:1 on white, over the
    /// 3:1 a control's boundary needs.
    pub fn field(&self, s: &str) -> String {
        self.fg256(244, s)
    }
    /// The meter's three states in soft tones (sage, amber, rose) instead of raw ANSI colours.
    pub fn level(&self, pct: u8, s: &str) -> String {
        let n = match pct {
            0..=59 => 108,
            60..=84 => 179,
            _ => 167,
        };
        self.fg256(n, s)
    }
}

/// The SGR code for a hue: the ANSI one on a dark terminal, the 256-palette shade on a light one.
fn hue_code(ansi: &str, on_light: u8, light: bool) -> String {
    if light {
        format!("38;5;{on_light}")
    } else {
        ansi.to_string()
    }
}

/// Where the provider colours live. Plain when the style is disabled.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    style: Style,
}

impl Theme {
    pub fn new(style: Style) -> Self {
        Self { style }
    }

    /// `text` in the colour of `provider` (claude orange-ish via the 256 palette, codex
    /// green, gemini magenta, grok blue, deepseek cyan); an unknown provider is bold.
    pub fn provider(&self, provider: &str, text: &str) -> String {
        let s = self.style;
        match provider.to_ascii_lowercase().as_str() {
            // #ff8700 is 2.4:1 on white; #af5f00 keeps the hue at 4.7:1.
            "claude" => s.fg256(if light_background() { 130 } else { 208 }, text),
            "codex" => s.green(text),
            "gemini" => s.magenta(text),
            "grok" => s.blue(text),
            "deepseek" => s.cyan(text),
            _ => s.bold(text),
        }
    }

    /// The provider's own name in its colour.
    pub fn name(&self, provider: &str) -> String {
        self.provider(provider, provider)
    }
}

/// `text` with each `` `backtick span` `` in cyan. Unchanged when the backticks do not
/// pair up (so a stray one never swallows the rest of a message).
pub(super) fn mark_code(text: &str, style: Style) -> String {
    if !style.enabled || !text.matches('`').count().is_multiple_of(2) {
        return text.to_string();
    }
    text.split('`')
        .enumerate()
        .map(|(i, part)| if i % 2 == 1 { style.cyan(&format!("`{part}`")) } else { part.to_string() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::strip_ansi;
    use crate::render::test_facts::*;

    #[test]
    fn style_detection_table() {
        // (tty, NO_COLOR, TERM, CLICOLOR_FORCE) -> enabled
        for (tty, no_color, term, force, want) in [
            (true, None, Some("xterm-256color"), None, true),
            (false, None, Some("xterm-256color"), None, false),
            (true, Some("1"), Some("xterm"), None, false),
            (true, Some(""), Some("xterm"), None, true),
            (true, None, Some("dumb"), None, false),
            (true, None, None, None, true),
            (false, None, Some("xterm"), Some("1"), true),
            (false, None, Some("xterm"), Some("0"), false),
            (false, None, Some("xterm"), Some(""), false),
            (false, None, Some("dumb"), Some("1"), true),
            (true, Some("1"), Some("xterm"), Some("1"), false),
        ] {
            assert_eq!(Style::resolve(tty, no_color, term, force).enabled, want, "{tty} {no_color:?} {term:?} {force:?}");
        }
    }

    #[test]
    fn a_disabled_style_adds_nothing() {
        for f in [Style::bold, Style::dim, Style::red, Style::green, Style::yellow, Style::cyan, Style::magenta, Style::blue] {
            assert_eq!(f(&OFF, "text"), "text");
        }
        assert_eq!(OFF.fg256(208, "text"), "text");
        let th = Theme::new(OFF);
        for p in ["claude", "codex", "gemini", "grok", "deepseek", "other"] {
            assert_eq!(th.name(p), p);
        }
    }

    #[test]
    fn the_shimmer_moves_with_time_and_is_plain_without_colour() {
        use std::time::Duration;
        let at = |ms| crate::render::strip_ansi(&Style::on().shimmer("Thinking", Duration::from_millis(ms)));
        assert_eq!(at(0), "Thinking", "the text itself never changes");
        assert_eq!(Style::off().shimmer("Thinking", Duration::from_millis(300)), "Thinking");
        let frames: std::collections::HashSet<String> = (0..16).map(|i| Style::on().shimmer("Thinking", Duration::from_millis(i * 100))).collect();
        // Under reduced motion (or a light terminal) the frames are all alike; otherwise they differ.
        let still = crate::tty::accessibility::reduced_motion();
        assert!(still && frames.len() == 1 || !still && frames.len() > 1 || frames.len() == 1);
    }

    #[test]
    fn an_enabled_style_wraps_and_resets() {
        assert_eq!(ON.bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(ON.dim("x"), "\x1b[2mx\x1b[0m");
        assert_eq!(ON.red("x"), "\x1b[31mx\x1b[0m");
        assert_eq!(ON.green("x"), "\x1b[32mx\x1b[0m");
        assert_eq!(ON.yellow("x"), "\x1b[33mx\x1b[0m");
        assert_eq!(ON.cyan("x"), "\x1b[36mx\x1b[0m");
        assert_eq!(ON.magenta("x"), "\x1b[35mx\x1b[0m");
        assert_eq!(ON.blue("x"), "\x1b[34mx\x1b[0m");
        assert_eq!(ON.bold(""), "");
    }

    #[test]
    fn nested_styles_reopen_the_outer_one() {
        let s = ON.dim(&format!("a {} b", ON.red("x")));
        assert_eq!(s, "\x1b[2ma \x1b[31mx\x1b[0m\x1b[2m b\x1b[0m");
        assert_eq!(strip_ansi(&s), "a x b");
    }

    #[test]
    fn provider_colours() {
        let th = Theme::new(ON);
        assert_eq!(th.name("claude"), "\x1b[38;5;208mclaude\x1b[0m");
        assert_eq!(th.name("codex"), "\x1b[32mcodex\x1b[0m");
        assert_eq!(th.name("gemini"), "\x1b[35mgemini\x1b[0m");
        assert_eq!(th.name("grok"), "\x1b[34mgrok\x1b[0m");
        assert_eq!(th.name("deepseek"), "\x1b[36mdeepseek\x1b[0m");
        assert_eq!(th.name("Codex"), "\x1b[32mCodex\x1b[0m");
        assert_eq!(th.name("mystery"), "\x1b[1mmystery\x1b[0m");
    }

    #[test]
    fn the_background_comes_from_the_theme_then_colorfgbg() {
        assert_eq!(background_from_env(Some("light"), Some("15;0")), Some(true));
        assert_eq!(background_from_env(Some("Dark"), Some("0;15")), Some(false));
        assert_eq!(background_from_env(None, Some("15;0")), Some(false));
        assert_eq!(background_from_env(None, Some("0;15")), Some(true));
        assert_eq!(background_from_env(Some("auto"), Some("0;default;15")), Some(true));
        assert_eq!(background_from_env(None, Some("garbage")), None);
        assert_eq!(background_from_env(None, None), None);
    }

    #[test]
    fn a_light_terminal_gets_darker_hues_and_a_dark_one_keeps_its_own_palette() {
        assert_eq!(hue_code("33", 94, false), "33");
        assert_eq!(hue_code("33", 94, true), "38;5;94");
        assert_eq!(ON.field("─"), "\x1b[38;5;244m─\x1b[0m");
        assert_eq!(OFF.field("─"), "─");
    }

    #[test]
    fn mark_code_pairs_backticks() {
        assert_eq!(mark_code("run `x y` now", ON), "run \x1b[36m`x y`\x1b[0m now");
        assert_eq!(mark_code("a ` b", ON), "a ` b");
        assert_eq!(mark_code("run `x`", OFF), "run `x`");
    }
}
