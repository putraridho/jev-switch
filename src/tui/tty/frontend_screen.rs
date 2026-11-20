//! The terminal writes of the front end. Finished output is printed once and scrolls away
//! with the terminal's own scrollback; the live area under it (a [`Frame`]) is erased and
//! drawn again on every change. Erasing works by counting rows, which is why no frame row is
//! ever as wide as the terminal.
//!
//! Each redraw is one write inside the terminal's synchronized-update brackets, so terminals
//! that know them never show a half-drawn frame (others ignore the brackets).

use std::io::Write;

use crate::tui::views::frontend_view::Frame;
use crate::tui::render::display_width;

const SYNC_ON: &str = "\x1b[?2026h";
const SYNC_OFF: &str = "\x1b[?2026l";
const HIDE: &str = "\x1b[?25l";
const SHOW: &str = "\x1b[?25h";
/// The alternate screen, with the mouse reported (wheel scrolling) in the SGR encoding.
pub const FULL_ON: &str = "\x1b[?1049h\x1b[?1000h\x1b[?1006h";
/// Clear from the cursor to the end of the screen.
const CLEAR_DOWN: &str = "\x1b[J";

pub struct Screen<W: Write> {
    out: W,
    /// Colour is on: a reset closes every printed line, so a style cannot leak into the next.
    styled: bool,
    /// Display width of each row drawn last, to count them again after a resize.
    rows: Vec<usize>,
    /// The row of the live area the cursor was left on.
    cursor_row: usize,
    /// The frame on screen, to skip a redraw that changes nothing and to rewrite only the rows
    /// that changed.
    last: Option<Frame>,
}

impl<W: Write> Screen<W> {
    pub fn new(out: W, styled: bool) -> Self {
        Self { out, styled, rows: Vec::new(), cursor_row: 0, last: None }
    }

    /// Take the alternate screen and the mouse (the full-screen layout).
    pub fn enter_full(&mut self) {
        self.last = None;
        self.write(&format!("{FULL_ON}\x1b[2J"));
    }

    /// Give the alternate screen and the mouse back.
    pub fn leave_full(&mut self) {
        self.last = None;
        self.write(&format!("{SHOW}{}", String::from_utf8_lossy(crate::tui::tty::terminal_guard::LEAVE_ALT_SCREEN)));
    }

    /// Draw a whole-terminal frame: every row at its own place, so nothing depends on where
    /// the cursor was. Only the rows that differ from the last frame are written.
    pub fn draw_full(&mut self, frame: &Frame) {
        if self.last.as_ref() == Some(frame) {
            return;
        }
        let mut buf = String::from(SYNC_ON);
        buf.push_str(HIDE);
        let same_shape = self.last.as_ref().is_some_and(|l| l.rows.len() == frame.rows.len());
        if !same_shape {
            buf.push_str("\x1b[2J");
        }
        for (i, row) in frame.rows.iter().enumerate() {
            if same_shape && self.last.as_ref().and_then(|l| l.rows.get(i)) == Some(row) {
                continue;
            }
            buf.push_str(&format!("\x1b[{};1H\x1b[2K{row}", i + 1));
            if self.styled {
                buf.push_str("\x1b[0m");
            }
        }
        if let Some((r, c)) = frame.cursor {
            buf.push_str(&format!("\x1b[{};{}H{SHOW}", r + 1, c + 1));
        }
        buf.push_str(SYNC_OFF);
        self.last = Some(frame.clone());
        self.write(&buf);
    }

    /// Back to the first row of the live area, at its first column.
    fn to_top(&self, buf: &mut String) {
        if self.cursor_row > 0 {
            buf.push_str(&format!("\x1b[{}A", self.cursor_row));
        }
        buf.push('\r');
    }

    fn write(&mut self, buf: &str) {
        let _ = self.out.write_all(buf.as_bytes());
        let _ = self.out.flush();
    }

    /// Remove the live area; the cursor is left where it started.
    pub fn erase(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let mut buf = String::new();
        self.to_top(&mut buf);
        buf.push_str(CLEAR_DOWN);
        buf.push_str(SHOW);
        self.rows.clear();
        self.cursor_row = 0;
        self.last = None;
        self.write(&buf);
    }

    /// Forget the live area without erasing it (the screen was cleared, or a command printed
    /// over it while it had the terminal).
    pub fn forget(&mut self) {
        self.rows.clear();
        self.cursor_row = 0;
        self.last = None;
    }

    /// The terminal is now `width` columns wide: rows drawn for a wider one have wrapped onto
    /// more screen rows, so count again how far up the live area starts.
    pub fn resized(&mut self, width: usize) {
        let width = width.max(1);
        let rows_on_screen = |w: usize| w.div_ceil(width).max(1);
        self.cursor_row = self.rows.iter().take(self.cursor_row).map(|w| rows_on_screen(*w)).sum();
        // The rows may have wrapped: the next draw must be a full one.
        self.last = None;
    }

    /// Print finished output above the live area (it scrolls into the scrollback), then
    /// draw `frame` under it.
    pub fn print_above(&mut self, text: &str, frame: &Frame) {
        let mut buf = String::from(SYNC_ON);
        self.to_top(&mut buf);
        buf.push_str(CLEAR_DOWN);
        // Raw mode: a line feed alone does not return to the first column.
        for line in text.split('\n') {
            buf.push_str(line);
            if self.styled {
                buf.push_str("\x1b[0m");
            }
            buf.push_str("\r\n");
        }
        self.rows.clear();
        self.cursor_row = 0;
        self.frame_into(&mut buf, frame);
        buf.push_str(SYNC_OFF);
        self.write(&buf);
    }

    /// Draw `frame` in place of the live area. Nothing is written when it is what is on screen
    /// already, and only the rows that changed when its shape is the same (a spinner tick
    /// rewrites one row, not the whole area).
    pub fn draw(&mut self, frame: &Frame) {
        if self.last.as_ref() == Some(frame) {
            return;
        }
        let same_shape = self.last.as_ref().is_some_and(|l| l.rows.len() == frame.rows.len() && l.cursor == frame.cursor) && !self.rows.is_empty();
        let mut buf = String::from(SYNC_ON);
        if same_shape {
            self.changed_rows_into(&mut buf, frame);
        } else {
            self.to_top(&mut buf);
            buf.push_str(CLEAR_DOWN);
            self.frame_into(&mut buf, frame);
        }
        buf.push_str(SYNC_OFF);
        self.write(&buf);
    }

    /// Rewrite the rows of `frame` that differ from the last one; the cursor ends where it was.
    fn changed_rows_into(&mut self, buf: &mut String, frame: &Frame) {
        let Some(last) = self.last.take() else { return };
        buf.push_str(HIDE);
        let mut at = self.cursor_row;
        for (i, (old, new)) in last.rows.iter().zip(&frame.rows).enumerate() {
            if old == new {
                continue;
            }
            match i.cmp(&at) {
                std::cmp::Ordering::Less => buf.push_str(&format!("\x1b[{}A", at - i)),
                std::cmp::Ordering::Greater => buf.push_str(&format!("\x1b[{}B", i - at)),
                std::cmp::Ordering::Equal => {}
            }
            at = i;
            buf.push_str("\r\x1b[2K");
            buf.push_str(new);
        }
        // Back to the cursor's place.
        match at.cmp(&self.cursor_row) {
            std::cmp::Ordering::Less => buf.push_str(&format!("\x1b[{}B", self.cursor_row - at)),
            std::cmp::Ordering::Greater => buf.push_str(&format!("\x1b[{}A", at - self.cursor_row)),
            std::cmp::Ordering::Equal => {}
        }
        buf.push('\r');
        if let Some((_, col)) = frame.cursor {
            if col > 0 {
                buf.push_str(&format!("\x1b[{col}C"));
            }
            buf.push_str(SHOW);
        }
        self.rows = frame.rows.iter().map(|r| display_width(r)).collect();
        self.last = Some(frame.clone());
    }

    fn frame_into(&mut self, buf: &mut String, frame: &Frame) {
        buf.push_str(HIDE);
        buf.push_str(&frame.rows.join("\r\n"));
        let last = frame.rows.len().saturating_sub(1);
        self.rows = frame.rows.iter().map(|r| display_width(r)).collect();
        self.last = Some(frame.clone());
        match frame.cursor {
            Some((row, col)) => {
                let row = row.min(last);
                if last > row {
                    buf.push_str(&format!("\x1b[{}A", last - row));
                }
                buf.push('\r');
                if col > 0 {
                    buf.push_str(&format!("\x1b[{col}C"));
                }
                buf.push_str(SHOW);
                self.cursor_row = row;
            }
            None => self.cursor_row = last,
        }
    }

    /// Write raw bytes (the bell, a screen clear).
    pub fn raw(&mut self, bytes: &str) {
        self.write(bytes);
    }

    #[cfg(test)]
    pub fn into_inner(self) -> W {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rows: &[&str], cursor: Option<(usize, usize)>) -> Frame {
        Frame { rows: rows.iter().map(|r| r.to_string()).collect(), cursor }
    }

    fn written(s: Screen<Vec<u8>>) -> String {
        String::from_utf8(s.into_inner()).unwrap_or_default()
    }

    #[test]
    fn a_full_frame_writes_each_row_in_place_and_a_tick_rewrites_only_what_changed() {
        let mut s = Screen::new(Vec::new(), false);
        s.draw_full(&frame(&["chat", "box", "foot"], Some((1, 2))));
        let before = s.out.len();
        s.draw_full(&frame(&["chat", "box", "foot"], Some((1, 2))));
        assert_eq!(s.out.len(), before, "nothing changed, nothing written");
        s.draw_full(&frame(&["chat", "box", "FOOT"], Some((1, 2))));
        let out = String::from_utf8(s.into_inner()).unwrap_or_default();
        assert!(out.starts_with("\x1b[?2026h\x1b[?25l\x1b[2J\x1b[1;1H\x1b[2Kchat\x1b[2;1H\x1b[2Kbox\x1b[3;1H\x1b[2Kfoot\x1b[2;3H\x1b[?25h"), "{out:?}");
        assert_eq!(&out[before..], "\x1b[?2026h\x1b[?25l\x1b[3;1H\x1b[2KFOOT\x1b[2;3H\x1b[?25h\x1b[?2026l");
    }

    #[test]
    fn a_redraw_goes_back_up_to_the_cursor_row_and_clears_down() {
        let mut s = Screen::new(Vec::new(), true);
        s.draw(&frame(&["work", "╭──╮", "│ › │", "╰──╯", "footer"], Some((2, 4))));
        s.draw(&frame(&["box"], None));
        let out = written(s);
        // The first frame: rows, then up 2 from the footer to the text row, column 4.
        assert!(out.contains("work\r\n╭──╮\r\n│ › │\r\n╰──╯\r\nfooter\x1b[2A\r\x1b[4C\x1b[?25h"), "{out:?}");
        // The second: up 2 rows (the cursor row) to the top, clear, draw.
        assert!(out.contains("\x1b[2A\r\x1b[J\x1b[?25lbox"), "{out:?}");
    }

    #[test]
    fn output_printed_above_ends_each_line_with_a_reset_and_a_carriage_return() {
        let mut s = Screen::new(Vec::new(), true);
        s.print_above("one\ntwo", &frame(&["box"], Some((0, 2))));
        let out = written(s);
        assert!(out.contains("\r\x1b[Jone\x1b[0m\r\ntwo\x1b[0m\r\n\x1b[?25lbox\r\x1b[2C"), "{out:?}");
    }

    #[test]
    fn an_unchanged_frame_writes_nothing_and_a_tick_rewrites_only_its_row() {
        let mut s = Screen::new(Vec::new(), true);
        let a = frame(&["⠋ work 1s", "╭──╮", "│ › │", "╰──╯", "footer"], Some((2, 4)));
        s.draw(&a);
        let before = s.out.len();
        s.draw(&a);
        assert_eq!(s.out.len(), before, "nothing changed, nothing written");
        s.draw(&frame(&["⠙ work 1s", "╭──╮", "│ › │", "╰──╯", "footer"], Some((2, 4))));
        let out = String::from_utf8(s.into_inner()).unwrap_or_default();
        let tick = &out[before..];
        // From the cursor row (2) up 2 to the working row, rewrite it, down 2, back to column 4.
        assert_eq!(tick, "\x1b[?2026h\x1b[?25l\x1b[2A\r\x1b[2K⠙ work 1s\x1b[2B\r\x1b[4C\x1b[?25h\x1b[?2026l");
    }

    #[test]
    fn after_a_resize_wrapped_rows_are_counted() {
        let mut s = Screen::new(Vec::new(), true);
        s.draw(&frame(&["x".repeat(30).as_str(), "y".repeat(30).as_str(), "z"], Some((2, 0))));
        s.resized(10);
        s.erase();
        let out = written(s);
        assert!(out.ends_with("\x1b[6A\r\x1b[J\x1b[?25h"), "two 30-column rows take 3 rows each at 10 columns: {out:?}");
    }
}
