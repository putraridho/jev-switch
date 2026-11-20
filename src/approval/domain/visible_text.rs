//! Text made safe to print: nothing in it can move the cursor, clear the screen or hide text.

/// A run of this many spaces or more is shown as a count, so spaces cannot push the rest of
/// a line out of sight; shorter runs are shown as they are.
const SPACE_RUN: usize = 8;

/// Whether `c` takes no room on screen or reorders text, so it could hide something.
pub(super) fn is_invisible(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F
        | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F
        | 0xFEFF | 0xFFA0 | 0xFFF9..=0xFFFB | 0xE0000..=0xE0FFF)
}

/// The marker for one control character: its picture from the Unicode control pictures
/// block (`\r` is ␍, ESC is ␛), or its code point for those without one.
fn control_marker(c: char) -> String {
    match c as u32 {
        n @ 0..=0x1f => char::from_u32(0x2400 + n).map_or_else(String::new, String::from),
        0x7f => "\u{2421}".to_string(),
        n => format!("\u{2039}U+{n:04X}\u{203a}"),
    }
}

/// `text` (one line) made safe to print and impossible to misread: a tab is `⇥`, other
/// control characters (a carriage return, ESC) their pictures, invisible and non-ASCII
/// whitespace characters their code point in `‹ ›`, and a long run of spaces a count. Every
/// other character is kept, so what is printed is what would run.
pub fn visible(text: &str) -> String {
    fn flush(out: &mut String, spaces: &mut usize) {
        if *spaces >= SPACE_RUN {
            out.push_str(&format!("\u{2039}{spaces} spaces\u{203a}"));
        } else {
            out.extend(std::iter::repeat_n(' ', *spaces));
        }
        *spaces = 0;
    }
    let (mut out, mut spaces) = (String::with_capacity(text.len()), 0usize);
    for c in text.chars() {
        if c == ' ' {
            spaces += 1;
            continue;
        }
        flush(&mut out, &mut spaces);
        match c {
            '\t' => out.push('\u{21e5}'),
            c if c.is_control() => out.push_str(&control_marker(c)),
            c if c.is_whitespace() || is_invisible(c) => out.push_str(&format!("\u{2039}U+{:04X}\u{203a}", c as u32)),
            c => out.push(c),
        }
    }
    flush(&mut out, &mut spaces);
    out
}
