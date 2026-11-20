//! ANSI escape sequences: telling them from printable text, stripping them, and the
//! few cursor-control strings the status line needs.

/// The SGR reset.
pub(super) const RESET: &str = "\x1b[0m";

pub(crate) use crate::shared::text::{segs, Seg};
pub use crate::shared::text::{clean_text, strip_ansi};

/// Erase the current line and return to column 0.
pub fn clear_line() -> &'static str {
    "\r\x1b[2K"
}

/// Move the cursor up one row, keeping its column.
pub fn up_row() -> &'static str {
    "\x1b[1A"
}

pub fn save_cursor() -> &'static str {
    "\x1b7"
}

pub fn restore_cursor() -> &'static str {
    "\x1b8"
}

pub fn hide_cursor() -> &'static str {
    "\x1b[?25l"
}

pub fn show_cursor() -> &'static str {
    "\x1b[?25h"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::display_width;

    #[test]
    fn cursor_escapes() {
        assert_eq!(clear_line(), "\r\x1b[2K");
        assert_eq!(save_cursor(), "\x1b7");
        assert_eq!(restore_cursor(), "\x1b8");
        assert_eq!(hide_cursor(), "\x1b[?25l");
        assert_eq!(show_cursor(), "\x1b[?25h");
        // they take no columns
        assert_eq!(display_width(&format!("{}x{}", save_cursor(), restore_cursor())), 1);
        assert_eq!(display_width(&format!("{}x{}", hide_cursor(), show_cursor())), 1);
    }

}
