//! Terminal columns: how wide text is on screen, and cutting or padding it to fit.
//! Character widths come from the `unicode-width` crate (UAX #11 plus emoji
//! sequences); escape sequences and control characters count 0.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::ansi::{segs, Seg, RESET};

/// Narrowest width the layout formatters plan for.
pub const MIN_WIDTH: usize = 20;
/// Width assumed when the terminal's cannot be read.
pub const DEFAULT_WIDTH: usize = 80;

pub(super) fn clamp_width(width: usize) -> usize {
    width.max(MIN_WIDTH)
}

/// Columns of the terminal: the `COLUMNS` environment variable when it is a positive
/// number, else [`DEFAULT_WIDTH`]. There is no ioctl: `libc` is only a transitive
/// dependency here, and shells export `COLUMNS` (and update it) for interactive use, so
/// a binary that wants the live size should set it from its own resize handling.
pub fn terminal_width() -> usize {
    width_from_env(std::env::var("COLUMNS").ok().as_deref())
}

/// The pure half of [`terminal_width`].
pub fn width_from_env(columns: Option<&str>) -> usize {
    columns.and_then(|c| c.trim().parse::<usize>().ok()).filter(|w| *w > 0).unwrap_or(DEFAULT_WIDTH)
}

/// Columns one character takes on its own: 0 for control and combining characters (and
/// variation selectors, zero-width joiners), 2 for East Asian wide and emoji, else 1.
/// A character's width can depend on its neighbours (an emoji with a skin-tone
/// modifier is 2 columns, not 4), so measure text with [`display_width`].
pub fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// A running width measure. Text is measured in runs, because a character's width can
/// depend on its neighbours; an escape sequence or control character ends a run.
struct Run {
    committed: usize,
    text: String,
}

impl Run {
    fn new() -> Self {
        Self { committed: 0, text: String::new() }
    }

    /// Close the current run: what it measured counts for good.
    fn flush(&mut self) {
        self.committed += self.text.width();
        self.text.clear();
    }

    /// Columns so far if the run ended here.
    fn width(&self) -> usize {
        self.committed + self.text.width()
    }
}

/// Terminal columns `s` takes: ANSI escape sequences count 0, wide characters 2,
/// combining characters 0, a multi-character emoji (flag, skin tone, ZWJ sequence) as
/// Unicode says. Newlines and other control characters count 0, so call it per line.
pub fn display_width(s: &str) -> usize {
    let mut run = Run::new();
    for seg in segs(s) {
        match seg {
            Seg::Ch(c) if !c.is_control() => run.text.push(c),
            _ => run.flush(),
        }
    }
    run.width()
}

/// `s` cut to at most `width` columns with a trailing `…` when something was cut. Escape
/// sequences are kept (they take no columns) and a reset is appended after a cut so a
/// style never leaks past it. Width 0 gives an empty string.
pub fn truncate_to_width(s: &str, width: usize) -> String {
    if display_width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let budget = width - 1;
    let (mut out, mut run, mut styled) = (String::new(), Run::new(), false);
    for seg in segs(s) {
        match seg {
            Seg::Esc(e) => {
                styled = true;
                run.flush();
                out.push_str(e);
            }
            Seg::Ch(c) if c.is_control() => {
                run.flush();
                out.push(c);
            }
            Seg::Ch(c) => {
                run.text.push(c);
                if run.width() > budget {
                    break;
                }
                out.push(c);
            }
        }
    }
    out.push('…');
    if styled {
        out.push_str(RESET);
    }
    out
}

/// Plain `s` cut from the START to at most `width` columns, keeping the tail (what
/// matters in a path): `…/deep/file.rs`.
pub(super) fn truncate_start_to_width(s: &str, width: usize) -> String {
    if display_width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail = String::new();
    for c in s.chars().rev() {
        let mut longer = String::from(c);
        longer.push_str(&tail);
        if display_width(&longer) > width - 1 {
            break;
        }
        tail = longer;
    }
    format!("…{tail}")
}

/// `s` padded with spaces on the right to `width` columns (unchanged when wider).
pub(super) fn pad_to_width(s: &str, width: usize) -> String {
    let w = display_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::strip_ansi;

    #[test]
    fn display_width_table() {
        for (s, w) in [
            ("", 0),
            ("abc", 3),
            ("日本語", 6),
            ("a日b", 4),
            ("🚀", 2),
            ("e\u{301}", 1),
            ("\u{200d}", 0),
            ("\x1b[31mred\x1b[0m", 3),
            ("\x1b[1;38;5;208mx\x1b[0m", 1),
            ("\x1b]8;;http://x\x07link\x1b]8;;\x07", 4),
            ("\x1b7a\x1b8", 1),
            ("⠹", 1),
            ("✓", 1),
            ("⚠", 1),
            ("→", 1),
            ("한글", 4),
            ("\t", 0),
        ] {
            assert_eq!(display_width(s), w, "{s:?}");
        }
    }

    #[test]
    fn malformed_escapes_never_panic() {
        for s in ["\x1b", "\x1b[", "\x1b[31", "\x1b[é", "\x1b]abc", "\x1b\x1b[0m", "a\x1b", "\x1b日", "\x1b[3\u{301}"] {
            let _ = display_width(s);
            let _ = strip_ansi(s);
            for w in 0..6 {
                let _ = truncate_to_width(s, w);
            }
        }
        assert_eq!(strip_ansi("a\x1b[31mb\x1b[0mc"), "abc");
    }

    #[test]
    fn truncate_table() {
        assert_eq!(truncate_to_width("hello", 5), "hello");
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello world", 8), "hello w…");
        assert_eq!(truncate_to_width("hello", 0), "");
        assert_eq!(truncate_to_width("hello", 1), "…");
        // a wide char that would straddle the budget is left out
        assert_eq!(truncate_to_width("日本語", 4), "日…");
        assert_eq!(truncate_to_width("日本語", 5), "日本…");
        assert_eq!(truncate_to_width("日本語", 6), "日本語");
        // style is kept and closed
        let cut = truncate_to_width("\x1b[2mhello world\x1b[0m", 6);
        assert_eq!(cut, "\x1b[2mhello…\x1b[0m");
        assert_eq!(display_width(&cut), 6);
    }

    #[test]
    fn truncate_never_exceeds_width_for_any_input() {
        let samples = ["", "plain text here", "日本語のテキスト", "🚀🚀🚀 launch 🚀", "e\u{301}e\u{301}e\u{301}e\u{301}", "\x1b[31mred\x1b[0m and more text"];
        for s in samples {
            for w in 0..30 {
                let t = truncate_to_width(s, w);
                assert!(display_width(&t) <= w, "{s:?} at {w}: {t:?}");
            }
        }
    }

    #[test]
    fn truncate_start_keeps_the_tail() {
        assert_eq!(truncate_start_to_width("src/a.rs", 20), "src/a.rs");
        assert_eq!(truncate_start_to_width("src/deep/dir/file.rs", 12), "…dir/file.rs");
        assert_eq!(truncate_start_to_width("abc", 0), "");
    }

    #[test]
    fn emoji_sequences_measure_as_one_glyph() {
        // The old hand-written tables counted these per code point (family 6, heart 1).
        for (s, w) in [("👍🏽", 2), ("👨\u{200d}👩\u{200d}👧", 2), ("🇯🇵", 2), ("❤\u{fe0f}", 2), ("a👍🏽b", 4)] {
            assert_eq!(display_width(s), w, "{s:?}");
        }
        assert_eq!(char_width('日'), 2);
        assert_eq!(char_width('\u{301}'), 0);
        assert_eq!(char_width('\n'), 0);
        // truncation agrees with the measure, even across a sequence
        for s in ["👨\u{200d}👩\u{200d}👧 family", "❤\u{fe0f}❤\u{fe0f}❤\u{fe0f}", "👍🏽👍🏽👍🏽"] {
            for w in 0..14 {
                assert!(display_width(&truncate_to_width(s, w)) <= w, "{s:?} at {w}");
            }
        }
    }

    #[test]
    fn terminal_width_reads_columns() {
        assert_eq!(width_from_env(Some("120")), 120);
        assert_eq!(width_from_env(Some(" 90 ")), 90);
        for bad in [None, Some(""), Some("0"), Some("-3"), Some("wide")] {
            assert_eq!(width_from_env(bad), DEFAULT_WIDTH, "{bad:?}");
        }
    }
}
