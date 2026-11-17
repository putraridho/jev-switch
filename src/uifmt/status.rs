//! The in-progress status line and its spinner.

use std::time::Duration;

use super::style::Style;
use super::text::{human_duration, one_line};
use super::width::{display_width, truncate_to_width};

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

/// What was typed during a turn, for the end of the status line: `› fix the tes▏` and
/// `· 2 queued`. A long draft keeps its end (where the cursor is); new lines show as `⏎`.
/// At most `width` columns; empty when there is nothing to show.
pub fn draft_tail(draft: &str, queued: usize, width: usize, style: Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if queued > 0 {
        parts.push(style.dim(&format!("{queued} queued")));
    }
    if !draft.is_empty() {
        let flat: String = draft.chars().map(|c| if c == '\n' { '⏎' } else { c }).collect();
        let room = width.saturating_sub(display_width(&parts.join(" ")) + 4);
        let mut kept: Vec<char> = Vec::new();
        let mut used = 0;
        for c in flat.chars().rev() {
            let w = super::width::char_width(c);
            if used + w > room {
                break;
            }
            used += w;
            kept.push(c);
        }
        kept.reverse();
        let shown: String = kept.into_iter().collect();
        let cut = if shown.chars().count() < flat.chars().count() { "…" } else { "" };
        parts.push(format!("{} {cut}{shown}▏", style.cyan("›")));
    }
    truncate_to_width(&parts.join(" "), width)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_draft_keeps_its_end_and_counts_the_queue() {
        assert_eq!(draft_tail("", 0, 40, OFF), "");
        assert_eq!(draft_tail("fix it", 0, 40, OFF), "› fix it▏");
        assert_eq!(draft_tail("", 2, 40, OFF), "2 queued");
        assert_eq!(draft_tail("a\nb", 1, 40, OFF), "1 queued › a⏎b▏");
        let long = draft_tail("the beginning is cut and the end stays", 0, 16, OFF);
        assert!(long.ends_with("end stays▏") && long.contains('…') && display_width(&long) <= 16, "{long}");
    }

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
