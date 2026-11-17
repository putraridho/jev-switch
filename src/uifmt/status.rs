//! The in-progress status line and its spinner.

use std::time::Duration;

use super::style::Style;
use super::text::{human_duration, one_line};
use super::width::truncate_to_width;

/// Braille spinner frames.
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How long each spinner frame shows.
pub const SPINNER_FRAME_MS: u128 = 100;

#[derive(Debug, Clone, Default)]
pub struct Spinner {
    frame: usize,
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next frame, wrapping.
    pub fn tick(&mut self) -> &'static str {
        let f = SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()];
        self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
        f
    }

    /// The frame for a moment in time, so a status line is a pure function of the
    /// elapsed time and needs no spinner state.
    pub fn frame_for(elapsed: Duration) -> &'static str {
        SPINNER_FRAMES[(elapsed.as_millis() / SPINNER_FRAME_MS) as usize % SPINNER_FRAMES.len()]
    }
}

/// The in-progress line: `⠹ codex · gpt-6-luna · 12s · shell cargo test`. The spinner
/// frame follows `elapsed`; the activity (the last tool call) is optional. Cut to at
/// most `width` columns (wide characters counted as 2) with an ellipsis. The caller
/// prints it after [`clear_line`].
pub fn status_line(model_label: &str, elapsed: Duration, activity: Option<&str>, width: usize, style: Style) -> String {
    let mut parts = vec![one_line(model_label), format!("{}s", elapsed.as_secs())];
    if elapsed.as_secs() >= 60 {
        parts[1] = human_duration(elapsed.as_secs() * 1000);
    }
    if let Some(a) = activity.map(one_line).filter(|a| !a.is_empty()) {
        parts.push(a);
    }
    parts.retain(|p| !p.is_empty());
    let line = format!("{} {}", style.cyan(Spinner::frame_for(elapsed)), style.dim(&parts.join(" · ")));
    truncate_to_width(&line, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::display_width;
    use crate::uifmt::testutil::*;

    #[test]
    fn status_line_layout() {
        let s = status_line("codex · gpt-6-luna", Duration::from_secs(12), Some("shell cargo test"), 80, OFF);
        assert_eq!(s, "⠋ codex · gpt-6-luna · 12s · shell cargo test");
        assert_eq!(status_line("codex", Duration::from_millis(300), None, 80, OFF), "⠸ codex · 0s");
        assert_eq!(status_line("codex", Duration::from_secs(125), Some("  "), 80, OFF), "⠋ codex · 2m05s");
        let styled = status_line("codex", Duration::ZERO, Some("a"), 80, ON);
        assert_eq!(styled, "\x1b[36m⠋\x1b[0m \x1b[2mcodex · 0s · a\x1b[0m");
    }

    #[test]
    fn status_line_fits_every_width() {
        let cases = [
            ("codex · gpt-6-luna", Some("shell cargo test --all-targets -- --nocapture")),
            ("claude · opus", Some("Edit 日本語のファイル名/とても長い/パス.rs")),
            ("grok", Some("🚀🚀🚀 thinking 🚀🚀🚀")),
            ("日本語モデル", None),
        ];
        for (label, activity) in cases {
            for w in 0..60 {
                for style in [ON, OFF] {
                    let s = status_line(label, Duration::from_secs(7), activity, w, style);
                    assert!(display_width(&s) <= w, "{s:?} at {w}");
                    if !style.enabled {
                        no_esc(&s);
                    }
                }
            }
        }
        let s = status_line("codex", Duration::from_secs(1), Some("a very long activity description"), 20, OFF);
        assert!(s.ends_with('…'));
        assert_eq!(display_width(&s), 20);
        // a newline in the activity cannot break the single line
        assert!(!status_line("x", Duration::ZERO, Some("a\nb"), 80, OFF).contains('\n'));
    }

    #[test]
    fn spinner_frames_cycle() {
        let mut sp = Spinner::new();
        let first: Vec<&str> = (0..10).map(|_| sp.tick()).collect();
        assert_eq!(first, SPINNER_FRAMES.to_vec());
        assert_eq!(sp.tick(), SPINNER_FRAMES[0]);
        assert_eq!(Spinner::frame_for(Duration::ZERO), "⠋");
        assert_eq!(Spinner::frame_for(Duration::from_millis(250)), "⠹");
        assert_eq!(Spinner::frame_for(Duration::from_millis(1000)), "⠋");
        assert!(SPINNER_FRAMES.iter().all(|f| display_width(f) == 1));
    }
}
