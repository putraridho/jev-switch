//! Colour: whether to emit it, the SGR painters, and the per-provider palette.

use super::ansi::RESET;

pub fn stdout_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

pub fn stderr_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stderr())
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
        Self::resolve(stream_is_tty, var("NO_COLOR").as_deref(), var("TERM").as_deref(), var("CLICOLOR_FORCE").as_deref())
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
    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn italic(&self, s: &str) -> String {
        self.paint("3", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    pub fn blue(&self, s: &str) -> String {
        self.paint("34", s)
    }
    pub fn magenta(&self, s: &str) -> String {
        self.paint("35", s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.paint("36", s)
    }
    /// 256-colour foreground.
    pub fn fg256(&self, n: u8, s: &str) -> String {
        self.paint(&format!("38;5;{n}"), s)
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
            "claude" => s.fg256(208, text),
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
    use crate::uifmt::strip_ansi;
    use crate::uifmt::testutil::*;

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
    fn mark_code_pairs_backticks() {
        assert_eq!(mark_code("run `x y` now", ON), "run \x1b[36m`x y`\x1b[0m now");
        assert_eq!(mark_code("a ` b", ON), "a ` b");
        assert_eq!(mark_code("run `x`", OFF), "run `x`");
    }
}
