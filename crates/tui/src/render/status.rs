//! The in-progress status line and its spinner.

use std::time::Duration;

use crate::render::style::Style;
use crate::render::text::{human_duration, one_line};
use crate::render::width::{display_width, truncate_to_width};

/// Braille spinner frames.
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The one frame shown in place of the spinner when motion is reduced (`JEV_REDUCED_MOTION`).
pub const STILL_FRAME: &str = "•";

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
        if crate::tty::accessibility::reduced_motion() {
            return STILL_FRAME;
        }
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
    let model = parts.first().cloned().unwrap_or_default();
    let rest = parts.iter().skip(1).cloned().collect::<Vec<_>>().join(" · ");
    let line = if rest.is_empty() {
        format!("{} {}", style.cyan(Spinner::frame_for(elapsed)), style.bold(&model))
    } else {
        format!("{} {} {} {}", style.cyan(Spinner::frame_for(elapsed)), style.bold(&model), style.dim("·"), style.dim(&rest))
    };
    truncate_to_width(&line, width)
}

/// The field under the status line while a turn runs: `› fix the tes▏` while something is
/// typed, and a hint in its place when nothing is, so it is plain that a message can be typed
/// and queued. A long draft keeps its end (where the cursor is); new lines show as `⏎`; waiting
/// messages are counted at the end. At most `width` columns.
pub fn input_row(draft: &str, queued: usize, width: usize, style: Style) -> String {
    let prefix = format!("{} ", style.cyan("›"));
    let count = if queued > 0 { format!("  {}", style.dim(&format!("{queued} queued"))) } else { String::new() };
    let body = if draft.is_empty() {
        let hint = if queued > 0 { "Enter queues another · Esc stops the turn" } else { "type to queue the next message · Esc stops the turn" };
        format!("▏{}", style.dim(hint))
    } else {
        let flat: String = draft.chars().map(|c| if c == '\n' { '⏎' } else { c }).collect();
        // Room for what is typed: the row, less the prefix, the cursor mark and the count.
        let room = width.saturating_sub(2 + 1 + display_width(&count));
        let room = if display_width(&flat) > room { room.saturating_sub(1) } else { room };
        let mut kept: Vec<char> = Vec::new();
        let mut used = 0;
        for c in flat.chars().rev() {
            let w = crate::render::width::char_width(c);
            if used + w > room {
                break;
            }
            used += w;
            kept.push(c);
        }
        kept.reverse();
        let shown: String = kept.into_iter().collect();
        let cut = if shown.chars().count() < flat.chars().count() { "…" } else { "" };
        format!("{cut}{shown}▏")
    };
    truncate_to_width(&format!("{prefix}{body}{count}"), width)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_field_shows_a_hint_then_the_draft_keeps_its_end_and_counts_the_queue() {
        let empty = input_row("", 0, 80, OFF);
        assert_eq!(empty, "› ▏type to queue the next message · Esc stops the turn");
        assert_eq!(input_row("", 2, 80, OFF), "› ▏Enter queues another · Esc stops the turn  2 queued");
        assert_eq!(input_row("fix it", 0, 40, OFF), "› fix it▏");
        assert_eq!(input_row("a\nb", 1, 40, OFF), "› a⏎b▏  1 queued");
        let long = input_row("the beginning is cut and the end stays", 0, 20, OFF);
        assert!(long.ends_with("end stays▏") && long.contains('…') && display_width(&long) <= 20, "{long}");
        for w in 0..70 {
            for draft in ["", "x", "日本語のとても長いメッセージ", "a\nb\nc"] {
                for queued in [0, 3] {
                    let row = input_row(draft, queued, w, OFF);
                    assert!(display_width(&row) <= w && !row.contains('\n'), "{row:?} at {w}");
                }
            }
        }
    }

    use super::*;
    use crate::render::display_width;
    use crate::render::test_facts::*;

    #[test]
    fn status_line_layout() {
        let s = status_line("codex · gpt-6-luna", Duration::from_secs(12), Some("shell cargo test"), 80, OFF);
        assert_eq!(s, "⠋ codex · gpt-6-luna · 12s · shell cargo test");
        assert_eq!(status_line("codex", Duration::from_millis(300), None, 80, OFF), "⠸ codex · 0s");
        assert_eq!(status_line("codex", Duration::from_secs(125), Some("  "), 80, OFF), "⠋ codex · 2m05s");
        let styled = status_line("codex", Duration::ZERO, Some("a"), 80, ON);
        assert_eq!(crate::render::strip_ansi(&styled), "⠋ codex · 0s · a");
        assert!(styled.contains("\x1b[1mcodex\x1b[0m"));
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
