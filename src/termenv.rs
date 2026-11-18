//! What kind of terminal the session runs in, and how to behave in it. A terminal inside
//! tmux or screen, over ssh, a Linux virtual console or `TERM=dumb` each takes fewer of the
//! fancy escape sequences or sends them over a slower link, so the front end asks this module
//! instead of assuming a modern local terminal. Also the one place pasted text is cleaned.
//!
//! Pure: [`detect`] reads settings through a lookup function, so every case is a unit test.

use std::time::Duration;

/// A terminal multiplexer between the program and the real terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
    Tmux,
    Screen,
}

/// How much the terminal itself is trusted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `TERM=dumb` or no `TERM`: no escape sequences are safe.
    Dumb,
    /// The Linux virtual console: no keyboard protocol, a limited set of colours.
    LinuxConsole,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermEnv {
    pub multiplexer: Option<Multiplexer>,
    /// Over ssh: every redraw crosses the network.
    pub remote: bool,
    pub kind: Kind,
}

/// Read the terminal's nature from `TMUX`, `STY`, `SSH_CONNECTION`/`SSH_TTY` and `TERM`.
pub fn detect(lookup: &dyn Fn(&str) -> Option<String>) -> TermEnv {
    let set = |name: &str| lookup(name).is_some_and(|v| !v.trim().is_empty());
    let term = lookup("TERM").unwrap_or_default().trim().to_ascii_lowercase();
    let multiplexer = if set("TMUX") || term.starts_with("tmux") {
        Some(Multiplexer::Tmux)
    } else if set("STY") || term.starts_with("screen") {
        Some(Multiplexer::Screen)
    } else {
        None
    };
    let kind = match term.as_str() {
        "" | "dumb" | "unknown" => Kind::Dumb,
        "linux" => Kind::LinuxConsole,
        _ => Kind::Normal,
    };
    TermEnv { multiplexer, remote: set("SSH_CONNECTION") || set("SSH_TTY") || set("SSH_CLIENT"), kind }
}

impl TermEnv {
    /// Whether to ask the terminal for the keyboard protocol that tells Shift-Enter from
    /// Enter. A multiplexer may print the request instead of passing it on, and the Linux
    /// console and a dumb terminal do not know it; there Alt-Enter and `\` Enter still add a line.
    pub fn keyboard_protocol(&self) -> bool {
        self.multiplexer.is_none() && self.kind == Kind::Normal
    }

    /// How often the spinner and the clock are redrawn: slower over ssh, where each redraw is
    /// bytes on a possibly slow link.
    pub fn tick(&self, normal: Duration) -> Duration {
        if self.remote {
            normal.max(Duration::from_millis(250))
        } else {
            normal
        }
    }

    /// One line for `/doctor` and the bug report: what was detected and what it changes.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        match self.multiplexer {
            Some(Multiplexer::Tmux) => parts.push("inside tmux"),
            Some(Multiplexer::Screen) => parts.push("inside screen"),
            None => {}
        }
        if self.remote {
            parts.push("over ssh (slower redraw)");
        }
        match self.kind {
            Kind::Dumb => parts.push("dumb terminal"),
            Kind::LinuxConsole => parts.push("Linux console"),
            Kind::Normal => {}
        }
        if parts.is_empty() {
            "a plain local terminal".to_string()
        } else if self.keyboard_protocol() {
            parts.join(", ")
        } else {
            format!("{} (Shift-Enter is not told from Enter; use Alt-Enter)", parts.join(", "))
        }
    }
}

// --------------------------------------------------------------------------- paste

/// The longest paste taken as one message. A bigger one (a whole log file) would make every
/// redraw crawl; the rest is dropped and the person is told.
pub const MAX_PASTE_BYTES: usize = 256 * 1024;

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// What was made of a paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pasted {
    pub text: String,
    /// More than [`MAX_PASTE_BYTES`] was pasted; `text` is the beginning.
    pub truncated: bool,
}

/// Clean a pasted text: the bracketed-paste markers are removed (pasted text that contains
/// the end marker could otherwise end the paste early and run the rest as typed keys), and a
/// huge paste is cut at a character boundary.
pub fn sanitize_paste(raw: &str) -> Pasted {
    let text = raw.replace(PASTE_END, "").replace(PASTE_START, "");
    if text.len() <= MAX_PASTE_BYTES {
        return Pasted { text, truncated: false };
    }
    let mut end = MAX_PASTE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Pasted { text: text[..end].to_string(), truncated: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> TermEnv {
        detect(&|name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn a_plain_terminal_gets_everything() {
        let t = env(&[("TERM", "xterm-256color")]);
        assert_eq!(t, TermEnv { multiplexer: None, remote: false, kind: Kind::Normal });
        assert!(t.keyboard_protocol());
        assert_eq!(t.describe(), "a plain local terminal");
        assert_eq!(t.tick(Duration::from_millis(80)), Duration::from_millis(80));
    }

    #[test]
    fn tmux_and_screen_are_found_by_their_variables_or_their_term() {
        assert_eq!(env(&[("TERM", "xterm"), ("TMUX", "/tmp/tmux-1/default,1,0")]).multiplexer, Some(Multiplexer::Tmux));
        assert_eq!(env(&[("TERM", "tmux-256color")]).multiplexer, Some(Multiplexer::Tmux));
        assert_eq!(env(&[("TERM", "screen"), ("STY", "123.pts")]).multiplexer, Some(Multiplexer::Screen));
        assert!(!env(&[("TERM", "xterm"), ("TMUX", "")]).multiplexer.is_some(), "an empty variable is not tmux");
        assert!(!env(&[("TERM", "tmux")]).keyboard_protocol());
    }

    #[test]
    fn ssh_slows_the_redraw_but_keeps_the_keyboard_protocol() {
        let t = env(&[("TERM", "xterm"), ("SSH_CONNECTION", "1.2.3.4 22 5.6.7.8 22")]);
        assert!(t.remote && t.keyboard_protocol());
        assert_eq!(t.tick(Duration::from_millis(80)), Duration::from_millis(250));
        assert_eq!(t.tick(Duration::from_millis(500)), Duration::from_millis(500), "never faster than asked");
        assert!(t.describe().contains("ssh"));
    }

    #[test]
    fn a_dumb_or_unset_term_and_the_linux_console_have_no_keyboard_protocol() {
        assert_eq!(env(&[]).kind, Kind::Dumb);
        assert_eq!(env(&[("TERM", "dumb")]).kind, Kind::Dumb);
        assert_eq!(env(&[("TERM", "linux")]).kind, Kind::LinuxConsole);
        assert!(!env(&[("TERM", "linux")]).keyboard_protocol());
        assert!(env(&[("TERM", "linux")]).describe().contains("Alt-Enter"));
    }

    #[test]
    fn paste_markers_inside_a_paste_are_removed() {
        let p = sanitize_paste("ls\x1b[201~\nrm -rf x\x1b[200~y");
        assert_eq!(p, Pasted { text: "ls\nrm -rf xy".into(), truncated: false });
    }

    #[test]
    fn a_huge_paste_is_cut_on_a_character_boundary() {
        let raw = "é".repeat(MAX_PASTE_BYTES);
        let p = sanitize_paste(&raw);
        assert!(p.truncated && p.text.len() <= MAX_PASTE_BYTES && p.text.chars().all(|c| c == 'é'));
        assert!(!sanitize_paste("short").truncated);
    }
}
