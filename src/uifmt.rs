//! The presentation layer of `jev-code`: every string the terminal UI prints is built
//! here, as a pure function from plain facts to a `String`, so the binary only decides
//! WHERE to print and each wording, colour and width rule is a unit test.
//!
//! Conventions, the same for every formatter:
//! - Input structs hold plain strings and numbers, never engine types, so the engine
//!   can change without touching the look.
//! - A [`Style`] that is disabled produces zero escape bytes. Colour is an addition to
//!   a line that already reads fine without it.
//! - Results have no trailing newline (except [`answer_text`], which keeps the line
//!   structure of the text it was given).
//! - Formatters that take a `width` clamp it to at least [`MIN_WIDTH`] columns (a width
//!   of 0 usually means "unknown"); the raw helpers ([`truncate_to_width`],
//!   [`wrap_to_width`], [`status_line`]) honour the width exactly.
//! - Widths are terminal columns: wide characters count 2, combining marks 0, and ANSI
//!   escape sequences 0. There is no dependency for this; the tables are small and
//!   deliberately approximate for rare scripts.

use std::path::Path;
use std::time::Duration;

use crate::backend::{ErrorKind, ProviderId};

/// Narrowest width the layout formatters plan for.
pub const MIN_WIDTH: usize = 20;
/// Width assumed when the terminal's cannot be read.
pub const DEFAULT_WIDTH: usize = 80;

fn clamp_width(width: usize) -> usize {
    width.max(MIN_WIDTH)
}

// ================================================================ width helpers

/// One piece of text: an escape sequence (zero width) or a printable char.
enum Seg<'a> {
    Esc(&'a str),
    Ch(char),
}

struct Segs<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Segs<'a> {
    type Item = Seg<'a>;

    fn next(&mut self) -> Option<Seg<'a>> {
        let c = self.rest.chars().next()?;
        if c == '\x1b' {
            let n = escape_len(self.rest);
            let (esc, rest) = self.rest.split_at(n);
            self.rest = rest;
            Some(Seg::Esc(esc))
        } else {
            self.rest = &self.rest[c.len_utf8()..];
            Some(Seg::Ch(c))
        }
    }
}

fn segs(s: &str) -> Segs<'_> {
    Segs { rest: s }
}

/// Byte length of the escape sequence at the start of `s` (which starts with ESC).
/// Handles CSI (`ESC [ ... final`), OSC (`ESC ] ... BEL|ST`) and two-byte escapes
/// (`ESC 7`). A malformed or unterminated sequence ends where it stops making sense,
/// always on a char boundary.
fn escape_len(s: &str) -> usize {
    let b = s.as_bytes();
    match b.get(1) {
        None => 1,
        Some(b'[') => {
            let mut i = 2;
            while i < b.len() {
                match b[i] {
                    0x20..=0x3f => i += 1,
                    0x40..=0x7e => return i + 1,
                    _ => return i,
                }
            }
            b.len()
        }
        Some(b']') => {
            let mut i = 2;
            while i < b.len() {
                match b[i] {
                    0x07 => return i + 1,
                    0x1b if b.get(i + 1) == Some(&b'\\') => return i + 2,
                    _ => i += 1,
                }
            }
            b.len()
        }
        Some(_) => 1 + s[1..].chars().next().map_or(0, char::len_utf8),
    }
}

/// `s` without ANSI escape sequences.
pub fn strip_ansi(s: &str) -> String {
    segs(s)
        .filter_map(|seg| match seg {
            Seg::Ch(c) => Some(c),
            Seg::Esc(_) => None,
        })
        .collect()
}

/// A single plain line from untrusted text (a CLI message, a tool summary): escapes and
/// control characters removed, every whitespace run collapsed to one space.
fn one_line(s: &str) -> String {
    let plain: String = strip_ansi(s).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn in_ranges(u: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&u))
}

const ZERO_WIDTH: &[(u32, u32)] = &[
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x05BF, 0x05BF),
    (0x05C1, 0x05C2),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1160, 0x11FF),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x20D0, 0x20FF),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0x1F3FB, 0x1F3FF),
    (0xE0020, 0xE007F),
    (0xE0100, 0xE01EF),
];

const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x33FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F251),
    (0x1F300, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F7E0, 0x1F7EB),
    (0x1F90C, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
    (0x20000, 0x3FFFD),
];

/// Columns one character takes: 0 for control and combining characters (and variation
/// selectors, zero-width joiners), 2 for East Asian wide and emoji, else 1. A skin-tone
/// modifier counts 0 so an emoji with one is 2, not 4; a flag is two regional
/// indicators of 1 each. Not a full `wcwidth`: rare scripts are approximated.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || (0x7f..0xa0).contains(&u) || in_ranges(u, ZERO_WIDTH) {
        0
    } else if in_ranges(u, WIDE) {
        2
    } else {
        1
    }
}

/// Terminal columns `s` takes: ANSI escape sequences count 0, wide characters 2,
/// combining characters 0. Newlines count 0, so call it per line.
pub fn display_width(s: &str) -> usize {
    segs(s)
        .map(|seg| match seg {
            Seg::Ch(c) => char_width(c),
            Seg::Esc(_) => 0,
        })
        .sum()
}

const RESET: &str = "\x1b[0m";

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
    let (mut out, mut used, mut styled) = (String::new(), 0usize, false);
    for seg in segs(s) {
        match seg {
            Seg::Esc(e) => {
                styled = true;
                out.push_str(e);
            }
            Seg::Ch(c) => {
                let w = char_width(c);
                if used + w > budget {
                    break;
                }
                used += w;
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
fn truncate_start_to_width(s: &str, width: usize) -> String {
    if display_width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let w = char_width(c);
        if used + w > width - 1 {
            break;
        }
        used += w;
        tail.push(c);
    }
    tail.push('…');
    tail.into_iter().rev().collect()
}

/// `s` padded with spaces on the right to `width` columns (unchanged when wider).
fn pad_to_width(s: &str, width: usize) -> String {
    let w = display_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

fn is_url(word: &str) -> bool {
    word.contains("://") || word.starts_with("www.")
}

/// How a run of words is drawn. Only answer text uses more than `Plain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Span {
    Plain,
    Bold,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tok {
    text: String,
    span: Span,
}

/// An unbreakable run: pieces of different spans glued with no space between them
/// (`**bold**,` is bold text then a plain comma, one word).
type Word = Vec<Tok>;

fn push_char(word: &mut Word, c: char, span: Span) {
    match word.last_mut() {
        Some(t) if t.span == span => t.text.push(c),
        _ => word.push(Tok { text: c.to_string(), span }),
    }
}

fn word_width(w: &Word) -> usize {
    w.iter().map(|t| display_width(&t.text)).sum()
}

fn word_text(w: &Word) -> String {
    w.iter().map(|t| t.text.as_str()).collect()
}

/// Cut a word into pieces of at most `avail` columns.
fn split_word(word: &Word, avail: usize) -> Vec<Word> {
    let (mut out, mut cur, mut used): (Vec<Word>, Word, usize) = (Vec::new(), Vec::new(), 0);
    for t in word {
        for c in t.text.chars() {
            let w = char_width(c);
            if used + w > avail && used > 0 {
                out.push(std::mem::take(&mut cur));
                used = 0;
            }
            push_char(&mut cur, c, t.span);
            used += w;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Greedy word wrap into lines of at most `avail` columns. A URL is never cut (it may
/// overflow); any other word longer than a line is split by column.
fn wrap_words(words: &[Word], avail: usize) -> Vec<Vec<Word>> {
    let avail = avail.max(1);
    let (mut lines, mut cur, mut cw): (Vec<Vec<Word>>, Vec<Word>, usize) = (Vec::new(), Vec::new(), 0);
    for word in words {
        let pieces = if word_width(word) > avail && !is_url(&word_text(word)) {
            split_word(word, avail)
        } else {
            vec![word.clone()]
        };
        for piece in pieces {
            let pw = word_width(&piece);
            if cur.is_empty() {
                cw = pw;
                cur.push(piece);
            } else if cw + 1 + pw <= avail {
                cw += 1 + pw;
                cur.push(piece);
            } else {
                lines.push(std::mem::take(&mut cur));
                cw = pw;
                cur.push(piece);
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn leading_indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Plain `text` word-wrapped to `width` columns, one `String` per output line. Existing
/// newlines are kept; a line that already fits is returned as it is; a wrapped line
/// keeps its leading indentation on every row. URLs are never broken; other words wider
/// than the line are split by column. Plain text only: escape sequences are not
/// carried across rows.
pub fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if display_width(line) <= width {
            out.push(line.trim_end().to_string());
            continue;
        }
        let mut indent = leading_indent(line).replace('\t', "    ");
        if display_width(&indent) * 2 >= width {
            indent.clear();
        }
        let words: Vec<Word> =
            line.split_whitespace().map(|w| vec![Tok { text: w.to_string(), span: Span::Plain }]).collect();
        let avail = width - display_width(&indent);
        for row in wrap_words(&words, avail) {
            let body: Vec<String> = row.iter().map(word_text).collect();
            out.push(format!("{indent}{}", body.join(" ")));
        }
    }
    out
}

// ================================================================== tty and env

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

pub fn stdout_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

pub fn stderr_is_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stderr())
}

// ===================================================================== styling

/// Whether and how to colour. Disabled means NOTHING is added to any string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub enabled: bool,
}

impl Style {
    pub fn on() -> Self {
        Self { enabled: true }
    }

    pub fn off() -> Self {
        Self { enabled: false }
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
        Self { enabled }
    }

    /// `s` in SGR `code`. Resets after the span, and re-opens the style after any
    /// reset inside `s`, so a coloured word inside a dim line does not end the dimming.
    fn paint(&self, code: &str, s: &str) -> String {
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

    fn span(&self, span: Span, s: &str) -> String {
        match span {
            Span::Plain => s.to_string(),
            Span::Bold => self.bold(s),
            Span::Code => self.cyan(s),
        }
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
fn mark_code(text: &str, style: Style) -> String {
    if !style.enabled || !text.matches('`').count().is_multiple_of(2) {
        return text.to_string();
    }
    text.split('`')
        .enumerate()
        .map(|(i, part)| if i % 2 == 1 { style.cyan(&format!("`{part}`")) } else { part.to_string() })
        .collect()
}

// ================================================================ humanising

/// A token count the way people read it: `189`, `1.2K`, `45K`, `1.2M`.
pub fn human_count(n: u64) -> String {
    fn scaled(n: u64, unit: u64, suffix: &str) -> String {
        if n < unit * 10 {
            let tenths = (n * 10 + unit / 2) / unit;
            if tenths.is_multiple_of(10) {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            }
        } else {
            format!("{}{suffix}", (n + unit / 2) / unit)
        }
    }
    if n < 1000 {
        n.to_string()
    } else if (n + 500) / 1000 >= 1000 {
        scaled(n, 1_000_000, "M")
    } else if n < 1_000_000 {
        scaled(n, 1000, "K")
    } else {
        scaled(n, 1_000_000, "M")
    }
}

/// A duration the way people read it: `850ms`, `14s`, `2m05s`, `1h05m`.
pub fn human_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = (ms + 500) / 1000;
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ============================================================== route and turn

/// What the router decided for one attempt, as plain strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteFacts {
    pub provider: String,
    pub model: String,
    /// The task type, used when `reason` does not carry one.
    pub task: String,
    /// The engine's long reason, e.g. `task: explain (0.91) → claude sonnet-5-5; first turn`.
    pub reason: String,
    pub switched_provider: bool,
    pub dropped_tier: bool,
}

struct ParsedReason {
    task: Option<String>,
    why: String,
}

/// `explain (0.91)` → `explain`; `explain (0.31)` → `explain (unsure)`.
fn task_label(head: &str) -> String {
    let t = head.split(" → ").next().unwrap_or(head).trim();
    let Some((name, paren)) = t.split_once(" (") else { return t.to_string() };
    let paren = paren.trim_end_matches(')').trim();
    let tag = match paren.parse::<f64>() {
        Ok(v) if v < 0.5 => " (unsure)",
        Ok(_) => "",
        Err(_) if paren == "security rule" => " (rule)",
        Err(_) if paren.eq_ignore_ascii_case("jev unavailable") => " (no Jev)",
        Err(_) => "",
    };
    format!("{}{tag}", name.trim())
}

/// Drop `(0.91)`-style confidences from `s`, except a low one (< 0.5), which becomes
/// `(unsure)`. Other parentheses are kept.
fn soften_confidence(s: &str) -> String {
    let (mut out, mut rest) = (String::new(), s);
    while let Some(i) = rest.find('(') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find(')') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let inner = &after[..j];
        match inner.trim().parse::<f64>() {
            Ok(v) if inner.contains('.') => {
                if v < 0.5 {
                    out.push_str("(unsure)");
                } else {
                    out.truncate(out.trim_end().len());
                }
            }
            _ => {
                out.push('(');
                out.push_str(inner);
                out.push(')');
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// The word after `prefix` in `c` (`switched from claude (…)` → `claude`).
fn word_after<'a>(c: &'a str, prefix: &str) -> &'a str {
    let rest = c.strip_prefix(prefix).unwrap_or("");
    rest.split([' ', '(']).next().unwrap_or("")
}

/// One `; `-separated piece of the engine's reason in a few words.
fn short_clause(clause: &str) -> String {
    let c = clause.trim();
    if c == "first turn" {
        "first turn".into()
    } else if c.starts_with("pinned provider") || c.starts_with("pinned model") {
        "pinned".into()
    } else if c.starts_with("same task, warm session") {
        "warm session".into()
    } else if c.starts_with("switched from ") {
        format!("switched from {}", word_after(c, "switched from "))
    } else if c.starts_with("not confident enough to switch from ") {
        format!("unsure, staying on {}", word_after(c, "not confident enough to switch from "))
    } else if c.starts_with("holding ") {
        "holding for cache".into()
    } else if c.starts_with("stepped down") {
        "stepped down".into()
    } else if c.starts_with("upgrade within") {
        "upgraded".into()
    } else if let Some(rest) = c.strip_prefix("steered off ") {
        let pct = rest.split("plan ").nth(1).and_then(|p| p.split('%').next()).unwrap_or("");
        let who = rest.split([' ', '(']).next().unwrap_or("");
        if pct.is_empty() {
            format!("steered off {who}")
        } else {
            format!("steered off {who} (plan {pct}%)")
        }
    } else if c.starts_with("Jev unavailable") {
        "no Jev, keeping model".into()
    } else if c.contains("cannot serve this turn any more") {
        "model unavailable".into()
    } else if c.starts_with("orchestrating:") {
        "orchestrating".into()
    } else if c.starts_with("orchestration skipped") {
        "orchestration skipped".into()
    } else if let Some(i) = c.find(" (pin ").filter(|_| c.contains("ignored: routing is off")) {
        let before = soften_confidence(&c[..i]);
        if before.is_empty() {
            "routing off, pin ignored".into()
        } else {
            format!("{before} · routing off, pin ignored")
        }
    } else {
        soften_confidence(c)
    }
}

fn parse_reason(reason: &str) -> ParsedReason {
    let r = reason.trim();
    let (task, tail) = if let Some(rest) = r.strip_prefix("task:") {
        let rest = rest.trim_start();
        let (head, tail) = rest.split_once("; ").unwrap_or((rest, ""));
        (Some(task_label(head)), tail)
    } else if let Some((_, why)) = r.split_once(" — ") {
        (None, why)
    } else {
        (None, r)
    };
    let why = tail.split("; ").map(short_clause).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" · ");
    ParsedReason { task, why }
}

/// The engine's long route reason as a short tag: `task: explain (0.91) → claude
/// sonnet-5-5; first turn` becomes `explain · first turn`. Confidence decimals are
/// dropped unless low (below 0.5), which reads `(unsure)`.
pub fn short_reason(reason: &str) -> String {
    let p = parse_reason(reason);
    [p.task.unwrap_or_default(), p.why].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// The one-line route announcement: `→ codex · gpt-6-luna   simple_edit · pinned`.
pub fn route_line(f: &RouteFacts, style: Style) -> String {
    let th = Theme::new(style);
    let p = parse_reason(&f.reason);
    let mut tag: Vec<String> = Vec::new();
    match p.task {
        Some(t) => tag.push(t),
        None if !f.task.is_empty() => tag.push(f.task.clone()),
        None => {}
    }
    if !p.why.is_empty() {
        tag.push(p.why);
    }
    if f.switched_provider && !tag.iter().any(|t| t.contains("switched")) {
        tag.push("switched provider".into());
    }
    if f.dropped_tier {
        tag.push("lower tier".into());
    }
    let who = if f.model.is_empty() {
        th.name(&f.provider)
    } else {
        format!("{} {} {}", th.name(&f.provider), style.dim("·"), f.model)
    };
    let head = format!("{} {who}", style.dim("→"));
    if tag.is_empty() {
        head
    } else {
        format!("{head}   {}", style.dim(&tag.join(" · ")))
    }
}

// ----------------------------------------------------------------- failures

/// What one failure kind says, in each place it is shown. One function per kind below
/// fills it in, so the wording of a kind lives in one place.
struct KindText {
    /// The inline sentence for [`failover_note`].
    inline: String,
    /// What to do next, in one clause, shown after the sentence.
    hint: Option<String>,
    /// The heading of an [`error_block`].
    block_title: &'static str,
    /// The "Try:" list.
    steps: Vec<String>,
}

/// A reset time worth printing: the CLI's own wording, not an epoch number.
fn reset_label(resets_at: Option<&str>) -> Option<String> {
    let r = one_line(resets_at?);
    (!r.is_empty() && !r.chars().all(|c| c.is_ascii_digit())).then_some(r)
}

/// How to log in to `provider`, from the one table in [`crate::doctor`].
fn login_hint(provider: &str) -> String {
    match provider.parse::<ProviderId>() {
        Ok(p) => crate::doctor::login_instruction(p),
        Err(_) => format!("log in to {provider}"),
    }
}

fn unusable_here_hint(provider: &str) -> String {
    match provider {
        "codex" => "it needs a git repository here (run `git init`, or work in a repo)".into(),
        "gemini" => "it needs a trusted folder (run `gemini` here once and trust it)".into(),
        _ => "it refuses to run in this directory".into(),
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn quota_text(resets_at: Option<&str>, provider: &str, th: &Theme) -> KindText {
    let when = reset_label(resets_at);
    let inline = match &when {
        Some(w) => format!("{} hit its limit (resets {w})", th.name(provider)),
        None => format!("{} hit its limit", th.name(provider)),
    };
    let wait = match &when {
        Some(w) => format!("Wait for the limit to reset ({w}) and send the turn again"),
        None => "Wait for the limit to reset, then send the turn again".into(),
    };
    KindText {
        inline,
        hint: None,
        block_title: "Usage limit reached",
        steps: vec![wait, "Use another provider now: `/model auto` or `/model <provider>`".into(), "See what is ready: `/providers`".into()],
    }
}

fn entitlement_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} is not on your plan", th.name(provider)),
        hint: None,
        block_title: "Model not on your plan",
        steps: vec![
            "Let the router pick a model your plan includes: `/model auto`".into(),
            format!("Check what {provider} offers you: `/providers`"),
        ],
    }
}

fn auth_text(provider: &str, th: &Theme) -> KindText {
    let hint = login_hint(provider);
    let need = if provider == "deepseek" { "needs a valid API key" } else { "needs a login" };
    KindText {
        inline: format!("{} {need}", th.name(provider)),
        steps: vec![capitalised(&hint), "Then check it with `/providers refresh`".into()],
        hint: Some(hint),
        block_title: "Login needed",
    }
}

fn empty_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} gave an empty answer", th.name(provider)),
        hint: None,
        block_title: "Empty answer",
        steps: vec!["Send it again".into(), "Or pick another model: `/model <provider>`".into()],
    }
}

fn stalled_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} stopped answering", th.name(provider)),
        hint: None,
        block_title: "Stopped answering",
        steps: vec![
            "Send it again, or split the task into smaller steps".into(),
            "Raise `CODE_IDLE_TIMEOUT_SECS` if the work is just slow".into(),
        ],
    }
}

fn unusable_here_text(provider: &str, th: &Theme) -> KindText {
    let hint = unusable_here_hint(provider);
    KindText {
        inline: format!("{} can't run in this directory", th.name(provider)),
        steps: vec![capitalised(&hint), "Or use another provider: `/model auto`".into()],
        hint: Some(hint),
        block_title: "Can't run in this directory",
    }
}

fn other_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} failed", th.name(provider)),
        hint: None,
        block_title: "The turn failed",
        steps: vec!["Send it again".into(), "If it keeps failing, check the setup: `jev-code doctor`".into()],
    }
}

fn kind_text(kind: &ErrorKind, provider: &str, model: &str, th: &Theme) -> KindText {
    match kind {
        ErrorKind::Quota { resets_at } => quota_text(resets_at.as_deref(), provider, th),
        ErrorKind::NotEntitled => entitlement_text(provider, model, th),
        ErrorKind::Auth => auth_text(provider, th),
        ErrorKind::Empty => empty_text(provider, model, th),
        ErrorKind::Stalled => stalled_text(provider, model, th),
        ErrorKind::UnusableHere => unusable_here_text(provider, th),
        ErrorKind::Other => other_text(provider, model, th),
    }
}

/// The "Try:" steps that fit `kind` for `provider`, for [`error_block`].
pub fn next_steps_for(kind: &ErrorKind, provider: &str) -> Vec<String> {
    kind_text(kind, provider, "", &Theme::new(Style::off())).steps
}

/// The note printed when a model failed and the turn moves on:
/// `⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol`, or, with nothing
/// to move to (`to` is `None`), just the problem and what to do:
/// `⚠ codex needs a login: run `codex login``. `discarded` adds
/// `(partial output discarded)` for an attempt that had already shown output.
pub fn failover_note(
    from_provider: &str,
    model: &str,
    kind: &ErrorKind,
    to: Option<(&str, &str)>,
    discarded: bool,
    style: Style,
) -> String {
    let th = Theme::new(style);
    let k = kind_text(kind, from_provider, model, &th);
    let mut s = format!("{} {}", style.yellow("⚠"), k.inline);
    if let Some(h) = &k.hint {
        s.push_str(": ");
        s.push_str(&mark_code(h, style));
    }
    if let Some((p, m)) = to {
        s.push_str(&format!(" → trying {} · {m}", th.name(p)));
    }
    if discarded {
        s.push(' ');
        s.push_str(&style.dim("(partial output discarded)"));
    }
    s
}

const MAX_DETAIL_LINES: usize = 3;
const MAX_DETAIL_COLS: usize = 160;

/// Up to three trimmed, plain lines of a CLI message.
fn detail_lines(message: &str) -> Vec<String> {
    let msg = message.trim();
    let msg = msg.strip_prefix("[error]").map_or(msg, str::trim_start);
    let all: Vec<String> = msg.split('\n').map(one_line).filter(|l| !l.is_empty()).collect();
    let more = all.len() > MAX_DETAIL_LINES;
    let mut lines: Vec<String> = all.into_iter().take(MAX_DETAIL_LINES).map(|l| truncate_to_width(&l, MAX_DETAIL_COLS)).collect();
    if more {
        if let Some(last) = lines.last_mut() {
            last.push_str(" …");
        }
    }
    lines
}

/// A title, dim detail lines and a "Try:" list: the shape of every fatal message.
fn block(title: &str, details: &[String], steps: &[String], style: Style) -> String {
    let mut lines = vec![style.paint("1;31", &format!("✗ {title}"))];
    lines.extend(details.iter().map(|d| format!("  {}", style.dim(d))));
    if !steps.is_empty() {
        lines.push(format!("  {}", style.bold("Try:")));
        lines.extend(steps.iter().map(|s| format!("    • {}", mark_code(s, style))));
    }
    lines.join("\n")
}

/// A fatal turn error: a red title for the kind, the CLI's message dimmed (at most 3
/// lines, trimmed, control characters removed) and the steps to try (see
/// [`next_steps_for`]).
pub fn error_block(kind: &ErrorKind, message: &str, next_steps: &[String], style: Style) -> String {
    let title = kind_text(kind, "", "", &Theme::new(Style::off())).block_title;
    block(title, &detail_lines(message), next_steps, style)
}

// -------------------------------------------------------- providers (shared)

/// Whether a provider can be used, mirroring the availability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    Ready,
    NotInstalled,
    NotLoggedIn,
    NoKey,
    Unknown,
}

impl ProviderStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInstalled => "not installed",
            Self::NotLoggedIn => "not logged in",
            Self::NoKey => "no key",
            Self::Unknown => "unknown",
        }
    }
}

/// One provider as `/providers` and the no-route message show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailRow {
    pub provider: String,
    pub status: ProviderStatus,
    pub plan: Option<String>,
    /// Models the login lists; `None` means every model the router knows.
    pub models: Option<Vec<String>>,
    /// Why, in a few words, from the probe.
    pub detail: String,
    pub verified: bool,
    /// How long a usage-limit cooldown has left, e.g. `2h 10m`.
    pub cooldown: Option<String>,
}

impl AvailRow {
    pub fn new(provider: &str, status: ProviderStatus) -> Self {
        Self {
            provider: provider.into(),
            status,
            plan: None,
            models: None,
            detail: String::new(),
            verified: true,
            cooldown: None,
        }
    }
}

/// Why nothing could run, plainly, and the fix: which providers are ready, why each
/// other one is not, and the commands that help (`/providers`, `jev-code doctor`).
pub fn no_route_message(task: &str, rows: &[AvailRow], style: Style) -> String {
    let title = if task.trim().is_empty() {
        "Nothing can take this turn".to_string()
    } else {
        format!("Nothing can take this {} turn", one_line(task))
    };
    let ready: Vec<&str> = rows
        .iter()
        .filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none())
        .map(|r| r.provider.as_str())
        .collect();
    let mut details = vec![if ready.is_empty() {
        "No provider is ready to run it.".to_string()
    } else {
        format!("Ready now, but none could take it: {}", ready.join(", "))
    }];
    let mut steps = vec!["See why each one can't run: `/providers`".to_string()];
    let mut fixable = false;
    for r in rows {
        let name = &r.provider;
        match (&r.cooldown, r.status) {
            (Some(left), _) => {
                details.push(format!("{name}: cooling down after a usage limit ({left} left)"));
                steps.push(format!("Wait for {name}'s limit to reset, or use another provider"));
            }
            (None, ProviderStatus::Ready) => {}
            (None, ProviderStatus::NotLoggedIn) => {
                details.push(format!("{name}: not logged in"));
                steps.push(capitalised(&login_hint(name)));
                fixable = true;
            }
            (None, ProviderStatus::NoKey) => {
                details.push(format!("{name}: no API key"));
                steps.push(capitalised(&login_hint(name)));
                fixable = true;
            }
            (None, ProviderStatus::NotInstalled) => details.push(format!("{name}: not installed")),
            (None, ProviderStatus::Unknown) => details.push(format!("{name}: could not be checked")),
        }
    }
    if task.trim() == "security" {
        details.push("Security work never goes to deepseek or grok.".to_string());
    }
    if fixable {
        steps.push("Then run `/providers refresh` and send it again".to_string());
    }
    steps.push("Check the whole setup: `jev-code doctor`".to_string());
    block(&title, &details, &steps, style)
}

// ------------------------------------------------------------------ summary

/// What a finished turn looked like, for the one-line summary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SummaryFacts {
    pub ok: bool,
    pub cancelled: bool,
    pub provider: String,
    pub model: String,
    pub elapsed_ms: u64,
    pub in_tokens: u64,
    pub cached_tokens: u64,
    pub out_tokens: u64,
    /// (provider, fraction of its plan window used, 0..1)
    pub plan: Option<(String, f64)>,
    /// (files, lines added, lines removed)
    pub files_changed: Option<(usize, u32, u32)>,
}

/// The one dim line after a turn: `✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) ·
/// out 189 · claude plan 32% · 3 files +12 −4`; `■ stopped · 3s` when cancelled; `✗`
/// when it failed. A plan window at 80% or more is yellow.
pub fn turn_summary(f: &SummaryFacts, style: Style) -> String {
    let th = Theme::new(style);
    let time = human_duration(f.elapsed_ms);
    if f.cancelled {
        return format!("{} {}", style.yellow("■"), style.dim(&format!("stopped · {time}")));
    }
    let mark = if f.ok { style.green("✓") } else { style.red("✗") };
    let mut parts: Vec<String> = Vec::new();
    if f.provider.is_empty() {
        if !f.ok {
            parts.push("failed".into());
        }
    } else if f.model.is_empty() {
        parts.push(th.name(&f.provider));
    } else {
        parts.push(format!("{} · {}", th.name(&f.provider), f.model));
    }
    parts.push(time);
    if f.in_tokens > 0 || f.out_tokens > 0 {
        let cached = if f.cached_tokens > 0 { format!(" (cached {})", human_count(f.cached_tokens)) } else { String::new() };
        parts.push(format!("in {}{cached}", human_count(f.in_tokens)));
        parts.push(format!("out {}", human_count(f.out_tokens)));
    }
    if let Some((p, used)) = &f.plan {
        let pct = (used.clamp(0.0, 1.0) * 100.0).round() as u32;
        let text = format!("{p} plan {pct}%");
        parts.push(if pct >= 80 { style.yellow(&text) } else { text });
    }
    if let Some((n, added, removed)) = f.files_changed.filter(|(n, _, _)| *n > 0) {
        let lines = if added == 0 && removed == 0 { String::new() } else { format!(" +{added} −{removed}") };
        parts.push(format!("{}{lines}", plural(n, "file", "files")));
    }
    format!("{mark} {}", style.dim(&parts.join(" · ")))
}

// ------------------------------------------------------------------ changes

/// One changed file, from `git status`/`git diff --numstat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// `A`, `M`, `D`, `R` (or `?` for untracked, shown like `A`).
    pub status: char,
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// Most files [`changes_block`] lists before saying "… and N more".
pub const MAX_CHANGE_ROWS: usize = 8;

fn status_letter(c: char, style: Style) -> String {
    let s = c.to_string();
    match c {
        'A' | '?' => style.green(&s),
        'M' => style.yellow(&s),
        'D' => style.red(&s),
        'R' => style.cyan(&s),
        _ => style.dim(&s),
    }
}

/// The compact per-file list under the summary: coloured status letter, path (cut from
/// the left to fit), `+n −n`. At most [`MAX_CHANGE_ROWS`] files, then `… and N more`.
/// Empty string for no files. No line is wider than `width`.
pub fn changes_block(files: &[FileChange], style: Style, width: usize) -> String {
    if files.is_empty() {
        return String::new();
    }
    let width = clamp_width(width);
    let shown = &files[..files.len().min(MAX_CHANGE_ROWS)];
    let counts: Vec<(String, String)> = shown
        .iter()
        .map(|f| if f.added == 0 && f.removed == 0 { (String::new(), String::new()) } else { (format!("+{}", f.added), format!("−{}", f.removed)) })
        .collect();
    let plus_w = counts.iter().map(|(a, _)| display_width(a)).max().unwrap_or(0);
    let minus_w = counts.iter().map(|(_, r)| display_width(r)).max().unwrap_or(0);
    let counts_w = if plus_w == 0 { 0 } else { plus_w + 1 + minus_w };
    let fixed = 4 + if counts_w > 0 { 2 + counts_w } else { 0 };
    let budget = width.saturating_sub(fixed).max(1);
    let paths: Vec<String> = shown.iter().map(|f| truncate_start_to_width(&one_line(&f.path), budget)).collect();
    let path_w = paths.iter().map(|p| display_width(p)).max().unwrap_or(0);
    let mut lines: Vec<String> = Vec::new();
    for ((f, path), (plus, minus)) in shown.iter().zip(&paths).zip(&counts) {
        let mut line = format!("  {} {}", status_letter(f.status, style), pad_to_width(path, path_w));
        if counts_w > 0 {
            if plus.is_empty() {
                line.push_str(&" ".repeat(2 + counts_w));
            } else {
                let pad = " ".repeat(plus_w - display_width(plus));
                line.push_str(&format!("  {pad}{} {}", style.green(plus), style.red(minus)));
            }
        }
        lines.push(truncate_to_width(line.trim_end(), width));
    }
    if files.len() > shown.len() {
        lines.push(style.dim(&format!("  … and {} more", files.len() - shown.len())));
    }
    lines.join("\n")
}

/// A unified diff coloured: file headers bold, `+` lines green, `-` lines red, `@@`
/// hunks cyan. Line structure (including a trailing newline) is preserved; a disabled
/// style returns the text unchanged.
pub fn diff_colorize(text: &str, style: Style) -> String {
    if !style.enabled {
        return text.to_string();
    }
    const HEADERS: [&str; 8] = ["diff ", "index ", "--- ", "+++ ", "new file", "deleted file", "similarity ", "rename "];
    text.split('\n')
        .map(|line| {
            let body = line.strip_suffix('\r').unwrap_or(line);
            if HEADERS.iter().any(|h| body.starts_with(h)) {
                style.bold(line)
            } else if body.starts_with("@@") {
                style.cyan(line)
            } else if body.starts_with('+') {
                style.green(line)
            } else if body.starts_with('-') {
                style.red(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// --------------------------------------------------------------- tool lines

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || "/._-~+@%".contains(c)
}

/// `text` with absolute paths inside `project_dir` made relative to it (`/p/src/a.rs` →
/// `src/a.rs`, `/p` → `.`). A sibling directory that merely starts with the same name
/// (`/p2/x`) is left alone.
pub fn relativize_paths(text: &str, project_dir: &Path) -> String {
    let dir = project_dir.to_string_lossy();
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        return text.to_string();
    }
    let (mut out, mut last) = (String::new(), 0);
    for (i, _) in text.match_indices(dir) {
        if i < last || text[..i].chars().next_back().is_some_and(is_path_char) {
            continue;
        }
        let after = &text[i + dir.len()..];
        match after.chars().next() {
            Some('/') => {
                out.push_str(&text[last..i]);
                if after[1..].chars().next().is_none_or(|c| !is_path_char(c)) {
                    out.push('.');
                }
                last = i + dir.len() + 1;
            }
            Some(c) if is_path_char(c) => {}
            _ => {
                out.push_str(&text[last..i]);
                out.push('.');
                last = i + dir.len();
            }
        }
    }
    out.push_str(&text[last..]);
    out
}

/// One tool call, dim: `  ⏺ Edit src/a.rs`. Paths inside the project are relative,
/// whitespace and newlines are collapsed to single spaces, and the line is cut with an
/// ellipsis to `width`.
pub fn tool_line(summary: &str, project_dir: &Path, style: Style, width: usize) -> String {
    let body = relativize_paths(&one_line(summary), project_dir);
    style.dim(&truncate_to_width(&format!("  ⏺ {body}"), clamp_width(width)))
}

// ================================================================== commands

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Model,
    Session,
    Info,
}

/// One slash command: the single table that help, completion and hints all read.
struct Cmd {
    name: &'static str,
    /// The argument synopsis shown in `/help`.
    args: &'static str,
    group: Group,
    about: &'static str,
    /// An example line for `/help`; empty when the bare command says it all.
    example: &'static str,
    /// What the inline hint shows after the command.
    hint: &'static str,
}

const COMMANDS: [Cmd; 12] = [
    Cmd { name: "/model", args: "[target]", group: Group::Model, about: "Pin a model or provider, or `auto` to let the router choose", example: "/model codex:gpt-6.1-sol", hint: "auto|codex|claude:..." },
    Cmd { name: "/provider", args: "auto|off", group: Group::Model, about: "Route across providers (auto), or use Claude only (off)", example: "/provider off", hint: "auto|off" },
    Cmd { name: "/orchestrate", args: "[on|off]", group: Group::Model, about: "Let a planner split a big task across cheaper workers", example: "/orchestrate on", hint: "on|off" },
    Cmd { name: "/new", args: "", group: Group::Session, about: "Start a fresh session on every provider", example: "", hint: "" },
    Cmd { name: "/resume", args: "", group: Group::Session, about: "Pick up a previous session", example: "", hint: "" },
    Cmd { name: "/diff", args: "[file]", group: Group::Session, about: "Show what the agent changed, all files or one", example: "/diff src/main.rs", hint: "[file]" },
    Cmd { name: "/undo", args: "", group: Group::Session, about: "Undo the file changes of the last turn", example: "", hint: "" },
    Cmd { name: "/quit", args: "", group: Group::Session, about: "Exit (also /exit, or Ctrl-D)", example: "", hint: "" },
    Cmd { name: "/help", args: "", group: Group::Info, about: "Show this help", example: "", hint: "" },
    Cmd { name: "/status", args: "", group: Group::Info, about: "Show the project, routing, providers and the last turn", example: "", hint: "" },
    Cmd { name: "/providers", args: "[refresh]", group: Group::Info, about: "Show what was detected; `refresh` probes again", example: "/providers refresh", hint: "[refresh]" },
    Cmd { name: "/doctor", args: "", group: Group::Info, about: "Check the setup and show how to fix problems", example: "", hint: "" },
];

const KEYS: [(&str, &str); 5] = [
    ("Ctrl-C", "stop the running turn"),
    ("Ctrl-D", "exit (on an empty line)"),
    ("Alt-Enter", "newline inside your message"),
    ("↑ / ↓", "earlier and later messages"),
    ("Tab", "complete a command or its argument"),
];

fn usage(c: &Cmd) -> String {
    if c.args.is_empty() {
        c.name.to_string()
    } else {
        format!("{} {}", c.name, c.args)
    }
}

/// Every slash command name (`/model`, ...), from the one table that help, completion and
/// parsing share.
pub fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).collect()
}

/// `/diff [file]`: the command with its argument synopsis, or `None` for a name that is
/// not in the table.
pub fn command_usage(name: &str) -> Option<String> {
    COMMANDS.iter().find(|c| c.name == name).map(usage)
}

/// A boxed message in the shape of every fatal message: a red title, dim detail lines and
/// a "Try:" list of steps. For callers that have their own wording.
pub fn problem_block(title: &str, details: &[String], steps: &[String], style: Style) -> String {
    block(title, details, steps, style)
}

/// The `/help` screen: commands in three groups, each with a line and, where it takes
/// an argument, an example; then the keys.
pub fn help_text(style: Style) -> String {
    let col = COMMANDS.iter().map(|c| display_width(&usage(c))).max().unwrap_or(0) + 2;
    let mut lines: Vec<String> = Vec::new();
    for (group, title) in [(Group::Model, "Model & routing"), (Group::Session, "Session"), (Group::Info, "Info")] {
        lines.push(style.bold(title));
        for c in COMMANDS.iter().filter(|c| c.group == group) {
            lines.push(format!("  {}{}", style.cyan(&pad_to_width(&usage(c), col)), c.about));
            if !c.example.is_empty() {
                lines.push(format!("  {}{}", " ".repeat(col), style.dim(&format!("e.g. {}", c.example))));
            }
        }
        lines.push(String::new());
    }
    lines.push(style.bold("Keys"));
    let key_col = KEYS.iter().map(|(k, _)| display_width(k)).max().unwrap_or(0) + 2;
    for (k, what) in KEYS {
        lines.push(format!("  {}{what}", style.cyan(&pad_to_width(k, key_col))));
    }
    lines.join("\n")
}

// -------------------------------------------------------------------- status

/// A provider that is ready, for `/status`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyProvider {
    pub provider: String,
    pub plan: Option<String>,
    /// Fraction of its plan window used (0..1), when known.
    pub plan_used: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusFacts {
    pub project: String,
    pub routing_on: bool,
    /// How the pin prints: `auto`, `codex`, `claude:claude-opus-5-5`.
    pub pin: String,
    pub orchestrate: bool,
    pub ready: Vec<ReadyProvider>,
    /// (provider, time left) for each provider cooling down after a limit.
    pub cooldowns: Vec<(String, String)>,
    pub last_turn: Option<SummaryFacts>,
    /// User turns in this session.
    pub turns: usize,
}

const LABEL_COLS: usize = 13;

fn status_row(label: &str, value: &str, style: Style) -> String {
    format!("  {}{value}", style.dim(&pad_to_width(label, LABEL_COLS)))
}

fn on_off(on: bool, style: Style) -> String {
    if on {
        style.green("on")
    } else {
        "off".to_string()
    }
}

/// `/status`: project, routing, pin, orchestration, ready providers with plan use,
/// cooldowns, the last turn and how many turns the session has had.
pub fn status_text(f: &StatusFacts, style: Style) -> String {
    let th = Theme::new(style);
    let mut rows = vec![status_row("Project", &one_line(&f.project), style)];
    let routing = if f.routing_on { on_off(true, style) } else { format!("off {}", style.dim("(Claude only)")) };
    rows.push(status_row("Routing", &routing, style));
    rows.push(status_row("Model", &one_line(&f.pin), style));
    rows.push(status_row("Orchestrate", &on_off(f.orchestrate, style), style));
    if f.ready.is_empty() {
        rows.push(status_row("Providers", &style.yellow("none ready: see /providers or `jev-code doctor`"), style));
    }
    for (i, p) in f.ready.iter().enumerate() {
        let mut v = format!("{} {}", style.green("●"), th.name(&p.provider));
        if let Some(plan) = &p.plan {
            v.push_str(&format!("  {}", style.dim(&one_line(plan))));
        }
        if let Some(used) = p.plan_used {
            let pct = (used.clamp(0.0, 1.0) * 100.0).round() as u32;
            let text = format!("plan {pct}% used");
            v.push_str(&format!("  {}", if pct >= 80 { style.yellow(&text) } else { style.dim(&text) }));
        }
        rows.push(status_row(if i == 0 { "Providers" } else { "" }, &v, style));
    }
    for (i, (p, left)) in f.cooldowns.iter().enumerate() {
        let v = format!("{} {}", th.name(p), style.yellow(&format!("for {left}")));
        rows.push(status_row(if i == 0 { "Cooling down" } else { "" }, &v, style));
    }
    let last = f.last_turn.as_ref().map_or_else(|| style.dim("none yet"), |t| turn_summary(t, style));
    rows.push(status_row("Last turn", &last, style));
    let turns = if f.turns == 0 { "no turns yet".to_string() } else { plural(f.turns, "turn", "turns") };
    rows.push(status_row("Session", &turns, style));
    rows.join("\n")
}

// ----------------------------------------------------------------- providers

fn status_glyph(status: ProviderStatus, cooling: bool, style: Style) -> String {
    match (status, cooling) {
        (ProviderStatus::Ready, false) => style.green("●"),
        (ProviderStatus::Ready, true) => style.yellow("◐"),
        (ProviderStatus::NotLoggedIn | ProviderStatus::NoKey, _) => style.yellow("○"),
        (ProviderStatus::NotInstalled, _) => style.dim("✕"),
        (ProviderStatus::Unknown, _) => style.dim("?"),
    }
}

/// The providers on one line, each as its status glyph and name: `● claude  ○ codex  ✕ gemini`
/// (the same glyphs as [`providers_table`]), cut to `width`. For the startup banner.
pub fn provider_glyphs(rows: &[AvailRow], style: Style, width: usize) -> String {
    let th = Theme::new(style);
    let line = rows
        .iter()
        .map(|r| format!("{} {}", status_glyph(r.status, r.cooldown.is_some(), style), th.provider(&r.provider, &r.provider)))
        .collect::<Vec<_>>()
        .join("  ");
    truncate_to_width(&line, clamp_width(width))
}

/// `/providers`: one aligned row per provider: glyph (● ready, ○ not logged in, ✕ not
/// installed, ? unknown, ◐ ready but cooling down), name, plan, then the models (or why
/// it is not usable) cut to fit `width`, `(unverified)` when the probe was never checked
/// against the real CLI.
pub fn providers_table(rows: &[AvailRow], style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let th = Theme::new(style);
    let name_w = rows.iter().map(|r| display_width(&r.provider)).max().unwrap_or(0);
    let plans: Vec<String> = rows.iter().map(|r| r.plan.as_deref().map_or("-".to_string(), one_line)).collect();
    let plan_w = plans.iter().map(|p| display_width(p)).max().unwrap_or(1);
    let mut lines = Vec::new();
    for (r, plan) in rows.iter().zip(&plans) {
        let cooling = r.cooldown.is_some();
        let prefix = format!(
            "  {} {} {} ",
            status_glyph(r.status, cooling, style),
            th.provider(&r.provider, &pad_to_width(&r.provider, name_w)),
            style.dim(&pad_to_width(plan, plan_w)),
        );
        let main_plain = if r.status == ProviderStatus::Ready {
            r.models.as_ref().map_or("all models".to_string(), |m| one_line(&m.join(", ")))
        } else {
            let detail = one_line(&r.detail);
            if detail.is_empty() || detail == r.status.label() {
                r.status.label().to_string()
            } else {
                format!("{} — {detail}", r.status.label())
            }
        };
        let mut suffix_plain = String::new();
        if !r.verified {
            suffix_plain.push_str(" (unverified)");
        }
        if let Some(left) = &r.cooldown {
            suffix_plain.push_str(&format!(" · cooling {}", one_line(left)));
        }
        let room = width.saturating_sub(display_width(&prefix)).max(1);
        let suffix_w = display_width(&suffix_plain);
        let (main_fit, show_suffix) = if display_width(&main_plain) + suffix_w <= room {
            (main_plain, true)
        } else if room >= suffix_w + 6 {
            (truncate_to_width(&main_plain, room - suffix_w), true)
        } else {
            (truncate_to_width(&main_plain, room), false)
        };
        let main = match r.status {
            ProviderStatus::Ready => main_fit,
            ProviderStatus::NotLoggedIn | ProviderStatus::NoKey => style.yellow(&main_fit),
            _ => style.dim(&main_fit),
        };
        let mut line = format!("{prefix}{main}");
        if show_suffix {
            if !r.verified {
                line.push_str(&style.dim(" (unverified)"));
            }
            if let Some(left) = &r.cooldown {
                line.push_str(&style.yellow(&format!(" · cooling {}", one_line(left))));
            }
        }
        lines.push(truncate_to_width(&line, width));
    }
    lines.join("\n")
}

// -------------------------------------------------------------------- doctor

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Fail,
}

/// One finding of `jev-code doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorItem {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Commands (or slash commands) that fix it.
    pub fix: Vec<String>,
}

fn severity_mark(s: Severity, style: Style) -> String {
    match s {
        Severity::Ok => style.green("✓"),
        Severity::Info => style.cyan("ⓘ"),
        Severity::Warn => style.yellow("⚠"),
        Severity::Fail => style.red("✗"),
    }
}

/// The doctor's findings in the order given (callers put problems first): a mark and
/// title per item, the detail dimmed and wrapped under it, and each fix on its own
/// indented line in cyan (`$ codex login`; a `/slash` command has no `$`). A blank line
/// separates items that have more than a title.
pub fn doctor_report(items: &[DoctorItem], style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let title = truncate_to_width(&one_line(&it.title), width - 2);
        let title = if matches!(it.severity, Severity::Warn | Severity::Fail) { style.bold(&title) } else { title };
        lines.push(format!("{} {title}", severity_mark(it.severity, style)));
        let detail = it.detail.trim();
        if !detail.is_empty() {
            let clean = detail.lines().map(one_line).collect::<Vec<_>>().join("\n");
            lines.extend(wrap_to_width(&clean, width - 4).into_iter().map(|l| format!("    {}", style.dim(&l))));
        }
        for fix in &it.fix {
            let fix = one_line(fix);
            let cmd = if fix.starts_with('/') { fix } else { format!("$ {fix}") };
            lines.push(format!("    {}", style.cyan(&truncate_to_width(&cmd, width - 4))));
        }
        if (!detail.is_empty() || !it.fix.is_empty()) && i + 1 < items.len() {
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

/// A closing line for the report: `All good`, or the counts of problems and warnings.
pub fn doctor_summary(items: &[DoctorItem], style: Style) -> String {
    let count = |s: Severity| items.iter().filter(|i| i.severity == s).count();
    let (fails, warns) = (count(Severity::Fail), count(Severity::Warn));
    match (fails, warns) {
        (0, 0) => style.green("All good"),
        (f, 0) => style.red(&plural(f, "problem", "problems")),
        (0, w) => style.yellow(&plural(w, "warning", "warnings")),
        (f, w) => format!("{} · {}", style.red(&plural(f, "problem", "problems")), style.yellow(&plural(w, "warning", "warnings"))),
    }
}

// ============================================================== answer markdown

/// Remove escape sequences and control characters (except tab) from model text, so
/// nothing the model prints can drive the terminal.
fn sanitize_answer_line(line: &str) -> String {
    strip_ansi(line.trim_end_matches('\r')).chars().filter(|c| !c.is_control() || *c == '\t').collect()
}

fn expand_leading_tabs(line: &str) -> String {
    let rest = line.trim_start_matches('\t');
    format!("{}{rest}", "    ".repeat(line.len() - rest.len()))
}

fn push_tok(out: &mut Vec<Tok>, text: &str, span: Span) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(t) if t.span == span => t.text.push_str(text),
        _ => out.push(Tok { text: text.to_string(), span }),
    }
}

fn backtick_run(chars: &[char], at: usize) -> usize {
    chars[at..].iter().take_while(|c| **c == '`').count()
}

/// Start of the next run of exactly `run` backticks at or after `from`.
fn find_backtick_run(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut k = from;
    while k < chars.len() {
        if chars[k] == '`' {
            let m = backtick_run(chars, k);
            if m == run {
                return Some(k);
            }
            k += m;
        } else {
            k += 1;
        }
    }
    None
}

/// Start of a closing `**` for a bold span whose content starts at `from`.
fn find_bold_close(chars: &[char], from: usize) -> Option<usize> {
    if chars.get(from).is_none_or(|c| c.is_whitespace()) {
        return None;
    }
    (from + 1..chars.len().saturating_sub(1))
        .find(|&j| chars[j] == '*' && chars[j + 1] == '*' && !chars[j - 1].is_whitespace())
}

/// Light inline markdown: `code`, `**bold**`. Anything unmatched stays literal.
fn parse_inline(s: &str, base: Span) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let (mut out, mut buf, mut i) = (Vec::new(), String::new(), 0);
    while i < chars.len() {
        if chars[i] == '`' {
            let run = backtick_run(&chars, i);
            if let Some(j) = find_backtick_run(&chars, i + run, run) {
                let code: String = chars[i + run..j].iter().collect();
                if !code.trim().is_empty() {
                    push_tok(&mut out, &std::mem::take(&mut buf), base);
                    push_tok(&mut out, code.trim(), Span::Code);
                    i = j + run;
                    continue;
                }
            }
            buf.extend(std::iter::repeat_n('`', run));
            i += run;
        } else if base != Span::Bold && chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(j) = find_bold_close(&chars, i + 2) {
                push_tok(&mut out, &std::mem::take(&mut buf), base);
                let inner: String = chars[i + 2..j].iter().collect();
                for t in parse_inline(&inner, Span::Bold) {
                    push_tok(&mut out, &t.text, t.span);
                }
                i = j + 2;
            } else {
                buf.push_str("**");
                i += 2;
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    push_tok(&mut out, &buf, base);
    out
}

fn to_words(toks: &[Tok]) -> Vec<Word> {
    let (mut words, mut cur): (Vec<Word>, Word) = (Vec::new(), Vec::new());
    for t in toks {
        for c in t.text.chars() {
            if c.is_whitespace() {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                push_char(&mut cur, c, t.span);
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn render_words(row: &[Word], style: Style) -> String {
    row.iter()
        .map(|w| w.iter().map(|t| style.span(t.span, &t.text)).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(indent, marker, rest)` of a list item or quote line.
fn list_marker(line: &str) -> Option<(usize, &str, &str)> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    for m in ["- ", "* ", "+ ", "> "] {
        if let Some(rest) = trimmed.strip_prefix(m) {
            return Some((indent, &trimmed[..1], rest));
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    let after = &trimmed[digits..];
    if (1..=3).contains(&digits) && (after.starts_with(". ") || after.starts_with(") ")) {
        return Some((indent, &trimmed[..digits + 1], &after[2..]));
    }
    None
}

/// `(fence char, length, info string)` when `line` opens or closes a code fence.
fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start_matches(' ');
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    let info = t[n..].trim();
    (n >= 3 && !(c == '`' && info.contains('`'))).then_some((c, n, info))
}

/// Wrap `words` to `avail` columns and draw them, with `first` before the first row and
/// `rest` before the others.
fn draw_wrapped(words: &[Word], avail: usize, first: &str, rest: &str, style: Style) -> Vec<String> {
    wrap_words(words, avail)
        .iter()
        .enumerate()
        .map(|(i, row)| format!("{}{}", if i == 0 { first } else { rest }, render_words(row, style)))
        .collect()
}

/// Assistant text with LIGHT markdown for a styled terminal: `#` headings bold,
/// `**bold**`, inline `code` cyan, fenced code blocks dimmed and indented under their
/// language, list items kept with a hanging indent, quotes, and prose word-wrapped to
/// `width`. Code blocks, table rows and URLs are never wrapped or broken; any other
/// word wider than the line is split by column. An unclosed fence treats the rest as
/// code. Escape sequences and control characters in the text are removed. With a
/// disabled style the text is returned UNCHANGED, so pipes and logs stay raw.
pub fn answer_text(text: &str, style: Style, width: usize) -> String {
    if !style.enabled {
        return text.to_string();
    }
    let width = clamp_width(width);
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for raw in text.split('\n') {
        let line = expand_leading_tabs(&sanitize_answer_line(raw));
        if let Some((fc, fl)) = fence {
            match fence_of(&line) {
                Some((c, n, info)) if c == fc && n >= fl && info.is_empty() => fence = None,
                _ if line.trim().is_empty() => out.push(String::new()),
                _ => out.push(format!("    {}", style.dim(line.trim_end()))),
            }
            continue;
        }
        if let Some((c, n, info)) = fence_of(&line) {
            fence = Some((c, n));
            if let Some(lang) = info.split_whitespace().next() {
                out.push(format!("  {}", style.dim(lang)));
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            out.push(String::new());
        } else if trimmed.starts_with('|') {
            out.push(line.trim_end().to_string());
        } else if let Some(h) = heading_text(trimmed) {
            let words = to_words(&parse_inline(h, Span::Bold));
            out.extend(draw_wrapped(&words, width, "", "", style));
        } else if let Some((indent, marker, rest)) = list_marker(&line) {
            let indent = if indent * 2 >= width { 0 } else { indent };
            let first = format!("{}{marker} ", " ".repeat(indent));
            let hang = " ".repeat(display_width(&first));
            let words = to_words(&parse_inline(rest, Span::Plain));
            out.extend(draw_wrapped(&words, width.saturating_sub(display_width(&first)), &first, &hang, style));
        } else {
            let lead = line.len() - trimmed.len();
            let indent = if lead * 2 >= width { 0 } else { lead };
            let pad = " ".repeat(indent);
            let words = to_words(&parse_inline(trimmed, Span::Plain));
            out.extend(draw_wrapped(&words, width - indent, &pad, &pad, style));
        }
    }
    out.join("\n")
}

/// The text of a `# heading` line (1 to 6 hashes then a space), if it is one.
fn heading_text(trimmed: &str) -> Option<&str> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    let text = rest.trim().trim_end_matches('#').trim_end();
    ((1..=6).contains(&hashes) && !text.is_empty()).then_some(text)
}

// =============================================================== status line

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

/// Erase the current line and return to column 0.
pub fn clear_line() -> &'static str {
    "\r\x1b[2K"
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

// ======================================================= completion and hints

/// What the line editor knows when completing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompleteCtx {
    /// Providers that are ready (`codex`, `claude`).
    pub ready_providers: Vec<String>,
    /// Usable models as `provider:model`.
    pub models: Vec<String>,
    /// Paths changed in this session, for `/diff`.
    pub changed_files: Vec<String>,
}

/// One completion. `replacement` replaces the word being completed (the text from
/// [`completion_start`] to the cursor); `display` is what a menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub replacement: String,
    pub display: String,
}

/// Byte offset where the word under the cursor (the end of `line`) starts.
pub fn completion_start(line: &str) -> usize {
    line.rfind(char::is_whitespace).map_or(0, |i| i + line[i..].chars().next().map_or(1, char::len_utf8))
}

fn starts_with_ci(word: &str, prefix: &str) -> bool {
    word.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn commands_sorted() -> Vec<&'static Cmd> {
    let mut v: Vec<&Cmd> = COMMANDS.iter().collect();
    v.sort_by_key(|c| c.name);
    v
}

/// The words an argument of `command` may be.
fn arg_choices(command: &str, ctx: &CompleteCtx) -> Vec<String> {
    let words: Vec<&str> = match command {
        "/provider" => vec!["auto", "off"],
        "/orchestrate" => vec!["on", "off"],
        "/providers" => vec!["refresh"],
        "/model" => ["auto"]
            .into_iter()
            .chain(ctx.ready_providers.iter().map(String::as_str))
            .chain(ctx.models.iter().map(String::as_str))
            .chain(["sonnet", "opus", "haiku"])
            .collect(),
        "/diff" => ctx.changed_files.iter().map(String::as_str).filter(|f| !f.contains(char::is_whitespace)).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for w in words {
        if !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// Candidates for the word under the cursor (the end of `line`): slash commands for the
/// first word; for the first argument of a command, its choices: `/model` takes `auto`,
/// each ready provider, `provider:model`, and the Claude aliases; `/provider` `auto|off`;
/// `/orchestrate` `on|off`; `/providers` `refresh`; `/diff` the changed files. Matching
/// ignores case. Prose (a line not starting with `/`) completes nothing.
pub fn complete(line: &str, ctx: &CompleteCtx) -> Vec<Candidate> {
    if !line.starts_with('/') {
        return Vec::new();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let typing_next = line.ends_with(char::is_whitespace);
    if words.len() == 1 && !typing_next {
        let col = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
        return commands_sorted()
            .into_iter()
            .filter(|c| starts_with_ci(c.name, words[0]))
            .map(|c| Candidate { replacement: c.name.to_string(), display: format!("{:<col$}{}", c.name, c.about) })
            .collect();
    }
    let (arg_index, prefix) = if typing_next { (words.len() - 1, "") } else { (words.len() - 2, words[words.len() - 1]) };
    if arg_index != 0 {
        return Vec::new();
    }
    arg_choices(words[0], ctx)
        .into_iter()
        .filter(|w| starts_with_ci(w, prefix))
        .map(|w| Candidate { display: w.clone(), replacement: w })
        .collect()
}

/// The faint inline hint after the cursor, as plain text (the editor paints it dim): the
/// rest of the command when a prefix of one is typed (`/mod` → `el`), and the arguments
/// a complete command takes (`/model ` → `auto|codex|claude:...`; `/model` without the
/// space → ` auto|codex|claude:...`). Nothing once an argument is being typed.
pub fn hint(line: &str) -> Option<String> {
    if !line.starts_with('/') {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let typing_next = line.ends_with(char::is_whitespace);
    let cmds = commands_sorted();
    let args_hint = |c: &Cmd| {
        (!c.hint.is_empty()).then(|| if typing_next { c.hint.to_string() } else { format!(" {}", c.hint) })
    };
    match (words.as_slice(), typing_next) {
        ([w], false) => match cmds.iter().find(|c| c.name == *w) {
            Some(c) => args_hint(c),
            None => cmds.iter().find(|c| c.name.starts_with(w)).map(|c| c.name[w.len()..].to_string()),
        },
        ([w], true) => cmds.iter().find(|c| c.name == *w).and_then(|c| args_hint(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Style = Style { enabled: true };
    const OFF: Style = Style { enabled: false };

    fn no_esc(s: &str) {
        assert!(!s.contains('\x1b'), "escape bytes in {s:?}");
    }

    fn lines(s: &str) -> Vec<&str> {
        s.split('\n').collect()
    }

    fn quota() -> ErrorKind {
        ErrorKind::Quota { resets_at: Some("5pm".into()) }
    }

    fn all_kinds() -> Vec<ErrorKind> {
        vec![
            quota(),
            ErrorKind::Quota { resets_at: None },
            ErrorKind::NotEntitled,
            ErrorKind::Auth,
            ErrorKind::Empty,
            ErrorKind::Stalled,
            ErrorKind::UnusableHere,
            ErrorKind::Other,
        ]
    }

    // ---------------------------------------------------------------- width

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
    fn wrap_basic_and_indent() {
        assert_eq!(wrap_to_width("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap_to_width("short", 80), vec!["short"]);
        assert_eq!(wrap_to_width("", 10), vec![""]);
        assert_eq!(wrap_to_width("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap_to_width("  aaa bbb ccc", 9), vec!["  aaa bbb", "  ccc"]);
        // a deep indent that leaves no room is dropped rather than wrapping forever
        assert!(wrap_to_width(&format!("{}x y z", " ".repeat(30)), 10).iter().all(|l| display_width(l) <= 10));
    }

    #[test]
    fn wrap_never_breaks_urls_but_splits_other_long_words() {
        let url = "https://example.com/a/very/long/path/that/does/not/fit";
        let out = wrap_to_width(&format!("see {url} now"), 20);
        assert_eq!(out, vec!["see", url, "now"]);
        let out = wrap_to_width("abcdefghijklmnop", 5);
        assert_eq!(out, vec!["abcde", "fghij", "klmno", "p"]);
    }

    #[test]
    fn wrap_counts_columns_not_bytes() {
        let out = wrap_to_width("日本語 日本語 日本語", 7);
        assert_eq!(out, vec!["日本語", "日本語", "日本語"]);
        for w in 1..20 {
            for l in wrap_to_width("🚀 launch 日本語のテキスト and e\u{301}cole", w) {
                assert!(display_width(&l) <= w.max(2), "{l:?} at {w}");
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

    // ---------------------------------------------------------------- style

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

    // ------------------------------------------------------------ humanising

    #[test]
    fn counts_are_humanised() {
        for (n, s) in [
            (0, "0"),
            (189, "189"),
            (999, "999"),
            (1000, "1K"),
            (1234, "1.2K"),
            (9_960, "10K"),
            (45_000, "45K"),
            (45_678, "46K"),
            (999_400, "999K"),
            (999_999, "1M"),
            (1_000_000, "1M"),
            (1_200_000, "1.2M"),
            (45_300_000, "45M"),
        ] {
            assert_eq!(human_count(n), s, "{n}");
        }
    }

    #[test]
    fn durations_are_humanised() {
        for (ms, s) in [
            (0, "0ms"),
            (850, "850ms"),
            (999, "999ms"),
            (1000, "1s"),
            (14_000, "14s"),
            (14_400, "14s"),
            (59_400, "59s"),
            (59_600, "1m00s"),
            (125_000, "2m05s"),
            (3_599_000, "59m59s"),
            (3_900_000, "1h05m"),
        ] {
            assert_eq!(human_duration(ms), s, "{ms}");
        }
    }

    // ---------------------------------------------------------------- route

    #[test]
    fn short_reasons() {
        for (long, short) in [
            ("task: explain (0.91) → claude sonnet-5-5; first turn", "explain · first turn"),
            ("task: simple_edit (0.88) → codex gpt-6-luna; pinned provider codex", "simple_edit · pinned"),
            ("task: simple_edit (0.88) → codex gpt-6.1-sol; pinned model", "simple_edit · pinned"),
            ("task: explain (0.31) → claude sonnet-5-5; first turn", "explain (unsure) · first turn"),
            ("task: explain (0.50) → claude sonnet-5-5; first turn", "explain · first turn"),
            ("task: security (security rule) → claude opus-5-5; first turn", "security (rule) · first turn"),
            ("task: unclear (Jev unavailable) → claude sonnet-5-5; Jev unavailable, keeping the current model", "unclear (no Jev) · no Jev, keeping model"),
            ("task: feature (0.93) → codex gpt-6.1-sol; switched from claude (new task, confident)", "feature · switched from claude"),
            ("task: review (0.70) → claude opus-5-5; not confident enough to switch from claude (new task, confident)", "review · unsure, staying on claude"),
            ("task: docs (0.90) → claude sonnet-5-5; same task, warm session (same provider)", "docs · warm session"),
            ("task: docs (0.90) → claude sonnet-5-5; holding claude-opus-5-5 for the cache, easier turn 1/2", "docs · holding for cache"),
            ("task: docs (0.90) → claude sonnet-5-5; stepped down after 2 easier turns", "docs · stepped down"),
            ("task: feature (0.90) → claude opus-5-5; upgrade within claude", "feature · upgraded"),
            ("task: feature (0.90) → codex gpt-6.1-sol; switched from claude (new task, confident); steered off claude (plan 91% used)", "feature · switched from claude · steered off claude (plan 91%)"),
            ("task: feature (0.90) → codex x; first turn; orchestrating: codex x plans, Jev picks the workers", "feature · first turn · orchestrating"),
            ("task: feature (0.90) → codex x; first turn; orchestration skipped: no delegate binary", "feature · first turn · orchestration skipped"),
            ("task: feature (0.90) → codex x; claude-opus-5-5 cannot serve this turn any more", "feature · model unavailable"),
            ("claude sonnet-5-5 — tier 1 (pin codex ignored: routing is off, claude only)", "tier 1 · routing off, pin ignored"),
            ("claude sonnet-5-5 — everyday (0.77)", "everyday"),
            ("", ""),
            ("something else entirely", "something else entirely"),
        ] {
            assert_eq!(short_reason(long), short, "{long}");
        }
    }

    #[test]
    fn route_lines() {
        let facts = |provider: &str, model: &str, reason: &str| RouteFacts {
            provider: provider.into(),
            model: model.into(),
            reason: reason.into(),
            ..Default::default()
        };
        assert_eq!(
            route_line(&facts("codex", "gpt-6-luna", "task: simple_edit (0.88) → codex gpt-6-luna; pinned provider codex"), OFF),
            "→ codex · gpt-6-luna   simple_edit · pinned"
        );
        assert_eq!(
            route_line(&facts("claude", "sonnet-5-5", "task: explain (0.91) → claude sonnet-5-5; first turn"), OFF),
            "→ claude · sonnet-5-5   explain · first turn"
        );
        // no reason at all: just the model
        assert_eq!(route_line(&facts("grok", "grok-5", ""), OFF), "→ grok · grok-5");
        // the task field is used when the reason has none
        let mut f = facts("codex", "gpt", "everyday");
        f.task = "explain".into();
        f.switched_provider = true;
        f.dropped_tier = true;
        assert_eq!(route_line(&f, OFF), "→ codex · gpt   explain · everyday · switched provider · lower tier");
        // a reason that already says "switched" is not repeated
        let mut f = facts("codex", "gpt", "task: feature (0.9) → codex gpt; switched from claude (x)");
        f.switched_provider = true;
        assert_eq!(route_line(&f, OFF), "→ codex · gpt   feature · switched from claude");
        // provider only
        assert_eq!(route_line(&facts("codex", "", ""), OFF), "→ codex");
    }

    #[test]
    fn route_line_colours_the_provider_and_dims_the_reason() {
        let f = RouteFacts {
            provider: "codex".into(),
            model: "gpt".into(),
            reason: "task: explain (0.9) → codex gpt; first turn".into(),
            ..Default::default()
        };
        let s = route_line(&f, ON);
        assert!(s.contains("\x1b[32mcodex\x1b[0m"));
        assert!(s.contains("\x1b[2mexplain · first turn\x1b[0m"));
        assert_eq!(strip_ansi(&s), route_line(&f, OFF));
    }

    // ------------------------------------------------------------- failures

    #[test]
    fn failover_notes_per_kind() {
        let to = Some(("codex", "gpt-6.1-sol"));
        let cases: Vec<(ErrorKind, &str, &str, Option<(&str, &str)>, bool, &str)> = vec![
            (quota(), "claude", "opus", to, false, "⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol"),
            (ErrorKind::Quota { resets_at: None }, "claude", "opus", to, false, "⚠ claude hit its limit → trying codex · gpt-6.1-sol"),
            // an epoch number is not a time anyone can read
            (ErrorKind::Quota { resets_at: Some("1794853451".into()) }, "claude", "opus", None, false, "⚠ claude hit its limit"),
            (ErrorKind::NotEntitled, "claude", "opus", to, false, "⚠ claude:opus is not on your plan → trying codex · gpt-6.1-sol"),
            (ErrorKind::Auth, "codex", "gpt", None, false, "⚠ codex needs a login: run `codex login`"),
            (ErrorKind::Auth, "claude", "x", Some(("codex", "g")), false, "⚠ claude needs a login: run `claude auth login` → trying codex · g"),
            (ErrorKind::Auth, "deepseek", "x", None, false, "⚠ deepseek needs a valid API key: add `DEEPSEEK_API_KEY` to ~/.jev-router.env"),
            (ErrorKind::Empty, "codex", "gpt", to, false, "⚠ codex:gpt gave an empty answer → trying codex · gpt-6.1-sol"),
            (ErrorKind::Stalled, "codex", "gpt", to, false, "⚠ codex:gpt stopped answering → trying codex · gpt-6.1-sol"),
            (ErrorKind::UnusableHere, "codex", "gpt", Some(("claude", "sonnet")), false, "⚠ codex can't run in this directory: it needs a git repository here (run `git init`, or work in a repo) → trying claude · sonnet"),
            (ErrorKind::UnusableHere, "gemini", "x", None, false, "⚠ gemini can't run in this directory: it needs a trusted folder (run `gemini` here once and trust it)"),
            (ErrorKind::Other, "grok", "g5", Some(("claude", "sonnet")), false, "⚠ grok:g5 failed → trying claude · sonnet"),
            (quota(), "claude", "opus", to, true, "⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol (partial output discarded)"),
        ];
        for (kind, from, model, to, discarded, want) in cases {
            assert_eq!(failover_note(from, model, &kind, to, discarded, OFF), want, "{kind:?}");
        }
    }

    #[test]
    fn failover_note_styles_are_additive() {
        for kind in all_kinds() {
            for discarded in [false, true] {
                let styled = failover_note("claude", "opus", &kind, Some(("codex", "g")), discarded, ON);
                let plain = failover_note("claude", "opus", &kind, Some(("codex", "g")), discarded, OFF);
                assert_eq!(strip_ansi(&styled), plain);
                no_esc(&plain);
            }
        }
        let s = failover_note("codex", "g", &ErrorKind::Auth, None, false, ON);
        assert!(s.contains("\x1b[36m`codex login`\x1b[0m"), "{s:?}");
        assert!(s.starts_with("\x1b[33m⚠\x1b[0m"));
    }

    #[test]
    fn error_blocks() {
        let steps = next_steps_for(&ErrorKind::Auth, "codex");
        let s = error_block(&ErrorKind::Auth, "[error] codex gpt failed: 401 Unauthorized\n\n  token expired  \nthird\nfourth", &steps, OFF);
        assert_eq!(
            s,
            "✗ Login needed\n  codex gpt failed: 401 Unauthorized\n  token expired\n  third …\n  Try:\n    • Run `codex login`\n    • Then check it with `/providers refresh`"
        );
        // no steps, no "Try:"
        assert_eq!(error_block(&ErrorKind::Other, "boom", &[], OFF), "✗ The turn failed\n  boom");
        // empty message: just the title
        assert_eq!(error_block(&ErrorKind::Empty, "  \n ", &[], OFF), "✗ Empty answer");
    }

    #[test]
    fn error_block_is_safe_with_hostile_messages() {
        let msg = "\x1b[2J\x1b]0;pwned\x07bad\r\nline\x08two";
        for style in [ON, OFF] {
            let s = error_block(&ErrorKind::Other, msg, &[], style);
            let body: Vec<&str> = s.split('\n').skip(1).collect();
            for l in body {
                assert!(!strip_ansi(l).contains(|c: char| c.is_control()), "{l:?}");
            }
        }
        no_esc(&error_block(&ErrorKind::Other, msg, &[], OFF));
        let long = "x".repeat(500);
        let s = error_block(&ErrorKind::Other, &long, &[], OFF);
        assert!(display_width(s.split('\n').nth(1).unwrap()) <= MAX_DETAIL_COLS + 2);
    }

    #[test]
    fn every_kind_has_steps_and_a_title() {
        for kind in all_kinds() {
            let steps = next_steps_for(&kind, "codex");
            assert!(!steps.is_empty(), "{kind:?}");
            let s = error_block(&kind, "msg", &steps, OFF);
            no_esc(&s);
            assert!(s.starts_with("✗ "));
            assert!(s.contains("Try:"));
        }
    }

    fn rows() -> Vec<AvailRow> {
        let mut claude = AvailRow::new("claude", ProviderStatus::Ready);
        claude.plan = Some("max_5x".into());
        claude.cooldown = Some("2h 10m".into());
        let mut codex = AvailRow::new("codex", ProviderStatus::NotLoggedIn);
        codex.detail = "run codex login".into();
        let gemini = AvailRow::new("gemini", ProviderStatus::NotInstalled);
        let mut deepseek = AvailRow::new("deepseek", ProviderStatus::NoKey);
        deepseek.verified = true;
        let mut grok = AvailRow::new("grok", ProviderStatus::Unknown);
        grok.verified = false;
        vec![claude, codex, gemini, deepseek, grok]
    }

    #[test]
    fn no_route_explains_and_fixes() {
        let s = no_route_message("feature", &rows(), OFF);
        assert_eq!(
            s,
            "✗ Nothing can take this feature turn\n  No provider is ready to run it.\n  claude: cooling down after a usage limit (2h 10m left)\n  codex: not logged in\n  gemini: not installed\n  deepseek: no API key\n  grok: could not be checked\n  Try:\n    • See why each one can't run: `/providers`\n    • Wait for claude's limit to reset, or use another provider\n    • Run `codex login`\n    • Add `DEEPSEEK_API_KEY` to ~/.jev-router.env\n    • Then run `/providers refresh` and send it again\n    • Check the whole setup: `jev-code doctor`"
        );
        let sec = no_route_message("security", &rows(), OFF);
        assert!(sec.contains("Security work never goes to deepseek or grok."));
        assert!(!s.contains("Security work"));
        let none = no_route_message("", &[], OFF);
        assert!(none.starts_with("✗ Nothing can take this turn\n  No provider is ready"));
        assert!(none.contains("/providers") && none.contains("jev-code doctor"));
        // a ready provider that still could not take it (it failed this turn)
        let ready = vec![AvailRow::new("codex", ProviderStatus::Ready)];
        assert!(no_route_message("docs", &ready, OFF).contains("Ready now, but none could take it: codex"));
    }

    // -------------------------------------------------------------- summary

    fn facts() -> SummaryFacts {
        SummaryFacts {
            ok: true,
            provider: "codex".into(),
            model: "gpt-6-luna".into(),
            elapsed_ms: 14_000,
            in_tokens: 45_000,
            cached_tokens: 40_000,
            out_tokens: 189,
            plan: Some(("claude".into(), 0.32)),
            files_changed: None,
            ..Default::default()
        }
    }

    #[test]
    fn turn_summaries() {
        assert_eq!(
            turn_summary(&facts(), OFF),
            "✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) · out 189 · claude plan 32%"
        );
        let mut f = facts();
        f.files_changed = Some((3, 12, 4));
        assert_eq!(
            turn_summary(&f, OFF),
            "✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) · out 189 · claude plan 32% · 3 files +12 −4"
        );
        f.files_changed = Some((1, 0, 0));
        f.plan = None;
        f.cached_tokens = 0;
        assert_eq!(turn_summary(&f, OFF), "✓ codex · gpt-6-luna · 14s · in 45K · out 189 · 1 file");
        f.files_changed = Some((0, 0, 0));
        f.in_tokens = 0;
        f.out_tokens = 0;
        assert_eq!(turn_summary(&f, OFF), "✓ codex · gpt-6-luna · 14s");
        let cancelled = SummaryFacts { cancelled: true, elapsed_ms: 3_000, ..facts() };
        assert_eq!(turn_summary(&cancelled, OFF), "■ stopped · 3s");
        let failed = SummaryFacts { ok: false, elapsed_ms: 850, in_tokens: 0, out_tokens: 0, plan: None, ..facts() };
        assert_eq!(turn_summary(&failed, OFF), "✗ codex · gpt-6-luna · 850ms");
        let nothing = SummaryFacts { ok: false, elapsed_ms: 2_000, ..Default::default() };
        assert_eq!(turn_summary(&nothing, OFF), "✗ failed · 2s");
        let no_model = SummaryFacts { model: String::new(), ..facts() };
        assert!(turn_summary(&no_model, OFF).starts_with("✓ codex · 14s"));
    }

    #[test]
    fn turn_summary_colours() {
        let s = turn_summary(&facts(), ON);
        assert!(s.starts_with("\x1b[32m✓\x1b[0m "));
        assert!(s.contains("\x1b[2m"));
        assert_eq!(strip_ansi(&s), turn_summary(&facts(), OFF));
        // the dimming survives the coloured provider name inside it
        assert!(s.contains("\x1b[0m\x1b[2m · gpt-6-luna"));
        let hot = SummaryFacts { plan: Some(("claude".into(), 0.93)), ..facts() };
        assert!(turn_summary(&hot, ON).contains("\x1b[33mclaude plan 93%\x1b[0m"));
        assert!(turn_summary(&hot, OFF).ends_with("claude plan 93%"));
        let over = SummaryFacts { plan: Some(("claude".into(), 7.0)), ..facts() };
        assert!(turn_summary(&over, OFF).ends_with("claude plan 100%"));
        let stopped = SummaryFacts { cancelled: true, ..facts() };
        assert!(turn_summary(&stopped, ON).starts_with("\x1b[33m■\x1b[0m"));
        let bad = SummaryFacts { ok: false, ..facts() };
        assert!(turn_summary(&bad, ON).starts_with("\x1b[31m✗\x1b[0m"));
    }

    // -------------------------------------------------------------- changes

    fn fc(status: char, path: &str, added: u32, removed: u32) -> FileChange {
        FileChange { status, path: path.into(), added, removed }
    }

    #[test]
    fn changes_block_lists_and_aligns() {
        let files = vec![fc('M', "src/a.rs", 12, 4), fc('A', "src/very_long_name.rs", 3, 0), fc('D', "old.txt", 0, 20)];
        assert_eq!(
            changes_block(&files, OFF, 80),
            "  M src/a.rs               +12 −4\n  A src/very_long_name.rs   +3 −0\n  D old.txt                 +0 −20"
        );
        assert_eq!(changes_block(&[], OFF, 80), "");
        // binary: no counts, no trailing spaces
        assert_eq!(changes_block(&[fc('M', "logo.png", 0, 0)], OFF, 80), "  M logo.png");
    }

    #[test]
    fn changes_block_caps_at_eight() {
        let files: Vec<FileChange> = (0..11).map(|i| fc('M', &format!("f{i}.rs"), 1, 1)).collect();
        let s = changes_block(&files, OFF, 60);
        let ls = lines(&s);
        assert_eq!(ls.len(), MAX_CHANGE_ROWS + 1);
        assert_eq!(ls[MAX_CHANGE_ROWS], "  … and 3 more");
        assert!(!s.contains("f8.rs"));
        let exact: Vec<FileChange> = files.into_iter().take(8).collect();
        assert!(!changes_block(&exact, OFF, 60).contains("more"));
    }

    #[test]
    fn changes_block_respects_width() {
        let files = vec![
            fc('M', "src/some/really/deep/directory/structure/with/a/file_name.rs", 120, 400),
            fc('A', "日本語/ファイル.rs", 1, 0),
            fc('M', "a\x1b[31m.rs", 1, 1),
        ];
        for w in [MIN_WIDTH, 24, 30, 40, 60, 100] {
            for style in [ON, OFF] {
                for l in changes_block(&files, style, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w}");
                }
            }
        }
        let s = changes_block(&files[..1], OFF, 40);
        assert!(s.contains("…") && s.contains("file_name.rs"), "{s}");
        no_esc(&changes_block(&files, OFF, 40));
    }

    #[test]
    fn changes_block_colours_statuses() {
        let files = vec![fc('A', "a", 1, 0), fc('M', "m", 1, 0), fc('D', "d", 0, 1), fc('?', "u", 0, 0), fc('R', "r", 0, 0)];
        let s = changes_block(&files, ON, 60);
        assert!(s.contains("\x1b[32mA\x1b[0m"));
        assert!(s.contains("\x1b[33mM\x1b[0m"));
        assert!(s.contains("\x1b[31mD\x1b[0m"));
        assert!(s.contains("\x1b[32m?\x1b[0m"));
        assert!(s.contains("\x1b[36mR\x1b[0m"));
        assert!(s.contains("\x1b[32m+1\x1b[0m") && s.contains("\x1b[31m−0\x1b[0m"));
        assert_eq!(strip_ansi(&s), changes_block(&files, OFF, 60));
    }

    #[test]
    fn diff_colours() {
        let diff = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n context\n-old\n+new\n";
        let s = diff_colorize(diff, ON);
        let ls = lines(&s);
        assert_eq!(ls[0], "\x1b[1mdiff --git a/x b/x\x1b[0m");
        assert_eq!(ls[2], "\x1b[1m--- a/x\x1b[0m");
        assert_eq!(ls[3], "\x1b[1m+++ b/x\x1b[0m");
        assert_eq!(ls[4], "\x1b[36m@@ -1,2 +1,2 @@\x1b[0m");
        assert_eq!(ls[5], " context");
        assert_eq!(ls[6], "\x1b[31m-old\x1b[0m");
        assert_eq!(ls[7], "\x1b[32m+new\x1b[0m");
        assert!(s.ends_with('\n'), "a trailing newline survives");
        assert_eq!(strip_ansi(&s), diff);
        assert_eq!(diff_colorize(diff, OFF), diff);
        assert_eq!(diff_colorize("", ON), "");
        assert_eq!(diff_colorize("+a\r\n", ON), "\x1b[32m+a\r\x1b[0m\n");
    }

    // ----------------------------------------------------------- tool lines

    #[test]
    fn tool_lines() {
        let dir = Path::new("/home/me/proj");
        for (summary, want) in [
            ("Edit /home/me/proj/src/a.rs", "  ⏺ Edit src/a.rs"),
            ("Edit src/a.rs", "  ⏺ Edit src/a.rs"),
            ("Read /home/me/proj2/x.rs", "  ⏺ Read /home/me/proj2/x.rs"),
            ("Read /home/me/proj-old/x.rs", "  ⏺ Read /home/me/proj-old/x.rs"),
            ("Bash cd /home/me/proj && cargo test", "  ⏺ Bash cd . && cargo test"),
            ("Bash ls /home/me/proj/", "  ⏺ Bash ls ."),
            ("Grep foo --path=/home/me/proj/src", "  ⏺ Grep foo --path=src"),
            ("Edit /elsewhere/a.rs", "  ⏺ Edit /elsewhere/a.rs"),
            ("Bash echo a\n  b\tc", "  ⏺ Bash echo a b c"),
            ("Write /home/me/proj/a.rs and /home/me/proj/b.rs", "  ⏺ Write a.rs and b.rs"),
        ] {
            assert_eq!(tool_line(summary, dir, OFF, 80), want, "{summary}");
        }
        // the project dir with a trailing slash behaves the same; the root dir changes nothing
        assert_eq!(tool_line("Edit /p/a.rs", Path::new("/p/"), OFF, 80), "  ⏺ Edit a.rs");
        assert_eq!(tool_line("Edit /p/a.rs", Path::new("/"), OFF, 80), "  ⏺ Edit /p/a.rs");
    }

    #[test]
    fn tool_lines_are_cut_to_width() {
        let dir = Path::new("/p");
        let cmd = format!("Bash {}", "cargo test --all-targets ".repeat(10));
        for w in [MIN_WIDTH, 30, 50, 120] {
            let s = tool_line(&cmd, dir, ON, w);
            assert!(display_width(&s) <= w, "{w}");
            assert!(strip_ansi(&s).ends_with('…') || w >= 120);
        }
        assert_eq!(tool_line("Bash 日本語日本語日本語日本語日本語日本語", dir, OFF, 20), "  ⏺ Bash 日本語日本…");
        let hostile = tool_line("Bash \x1b[2Jrm", dir, OFF, 40);
        no_esc(&hostile);
        assert_eq!(hostile, "  ⏺ Bash rm");
    }

    #[test]
    fn tool_line_is_dim() {
        assert_eq!(tool_line("Edit a", Path::new("/p"), ON, 40), "\x1b[2m  ⏺ Edit a\x1b[0m");
    }

    // ----------------------------------------------------------------- help

    #[test]
    fn help_lists_every_command_and_key() {
        let s = help_text(OFF);
        no_esc(&s);
        for c in [
            "/help", "/status", "/providers [refresh]", "/doctor", "/model [target]", "/provider auto|off", "/orchestrate [on|off]",
            "/diff [file]", "/undo", "/resume", "/new", "/quit",
        ] {
            assert!(s.contains(c), "{c}\n{s}");
        }
        for k in ["Ctrl-C", "stop the running turn", "Ctrl-D", "Alt-Enter", "↑ / ↓", "Tab"] {
            assert!(s.contains(k), "{k}");
        }
        let ls = lines(&s);
        let at = |h: &str| ls.iter().position(|l| *l == h).unwrap_or_else(|| panic!("{h}"));
        assert!(at("Model & routing") < at("Session") && at("Session") < at("Info") && at("Info") < at("Keys"));
        // each command sits in its group
        let line_of = |c: &str| ls.iter().position(|l| l.trim_start().starts_with(c)).unwrap();
        assert!(line_of("/model") > at("Model & routing") && line_of("/model") < at("Session"));
        assert!(line_of("/undo") > at("Session") && line_of("/undo") < at("Info"));
        assert!(line_of("/doctor") > at("Info") && line_of("/doctor") < at("Keys"));
        assert!(s.contains("e.g. /model codex:gpt-6.1-sol"));
        assert_eq!(strip_ansi(&help_text(ON)), s);
        assert!(help_text(ON).contains("\x1b[1mKeys\x1b[0m"));
    }

    #[test]
    fn help_columns_align() {
        let s = help_text(OFF);
        let col = s.lines().find(|l| l.trim_start().starts_with("/new")).map(|l| l.find("Start").unwrap()).unwrap();
        let col2 = s.lines().find(|l| l.trim_start().starts_with("/model [target]")).map(|l| l.find("Pin").unwrap()).unwrap();
        assert_eq!(col, col2);
    }

    // --------------------------------------------------------------- status

    #[test]
    fn status_text_layout() {
        let f = StatusFacts {
            project: "/home/me/proj".into(),
            routing_on: true,
            pin: "auto".into(),
            orchestrate: false,
            ready: vec![
                ReadyProvider { provider: "claude".into(), plan: Some("max_5x".into()), plan_used: Some(0.32) },
                ReadyProvider { provider: "codex".into(), plan: None, plan_used: None },
            ],
            cooldowns: vec![("gemini".into(), "1h 5m".into())],
            last_turn: Some(SummaryFacts { ok: true, provider: "codex".into(), model: "g".into(), elapsed_ms: 2000, ..Default::default() }),
            turns: 4,
        };
        assert_eq!(
            status_text(&f, OFF),
            "  Project      /home/me/proj\n  Routing      on\n  Model        auto\n  Orchestrate  off\n  Providers    ● claude  max_5x  plan 32% used\n               ● codex\n  Cooling down gemini for 1h 5m\n  Last turn    ✓ codex · g · 2s\n  Session      4 turns"
        );
        let bare = StatusFacts { project: "/p".into(), pin: "codex".into(), turns: 1, ..Default::default() };
        assert_eq!(
            status_text(&bare, OFF),
            "  Project      /p\n  Routing      off (Claude only)\n  Model        codex\n  Orchestrate  off\n  Providers    none ready: see /providers or `jev-code doctor`\n  Last turn    none yet\n  Session      1 turn"
        );
        let fresh = StatusFacts { ..Default::default() };
        assert!(status_text(&fresh, OFF).ends_with("Session      no turns yet"));
        assert_eq!(strip_ansi(&status_text(&f, ON)), status_text(&f, OFF));
        let hot = StatusFacts { ready: vec![ReadyProvider { provider: "claude".into(), plan: None, plan_used: Some(0.9) }], ..Default::default() };
        assert!(status_text(&hot, ON).contains("\x1b[33mplan 90% used\x1b[0m"));
    }

    // ------------------------------------------------------------ providers

    #[test]
    fn providers_table_layout() {
        let s = providers_table(&rows(), OFF, 100);
        assert_eq!(
            s,
            "  ◐ claude   max_5x all models · cooling 2h 10m\n  ○ codex    -      not logged in — run codex login\n  ✕ gemini   -      not installed\n  ○ deepseek -      no key\n  ? grok     -      unknown (unverified)"
        );
        assert_eq!(providers_table(&[], OFF, 80), "");
    }

    #[test]
    fn providers_table_glyphs_and_models() {
        let mut ready = AvailRow::new("codex", ProviderStatus::Ready);
        ready.plan = Some("plus".into());
        ready.models = Some(vec!["gpt-6.1-sol".into(), "gpt-6-luna".into()]);
        let s = providers_table(&[ready.clone()], OFF, 80);
        assert_eq!(s, "  ● codex plus gpt-6.1-sol, gpt-6-luna");
        let mut unverified = ready;
        unverified.verified = false;
        assert!(providers_table(&[unverified], OFF, 80).ends_with("gpt-6-luna (unverified)"));
        let styled = providers_table(&rows(), ON, 100);
        assert!(styled.contains("\x1b[33m◐\x1b[0m"));
        assert!(styled.contains("\x1b[33m○\x1b[0m"));
        assert!(styled.contains("\x1b[2m✕\x1b[0m"));
        assert!(styled.contains("\x1b[2m?\x1b[0m"));
        assert!(styled.contains("\x1b[2m (unverified)\x1b[0m"));
        assert_eq!(strip_ansi(&styled), providers_table(&rows(), OFF, 100));
        let all = AvailRow::new("claude", ProviderStatus::Ready);
        assert!(providers_table(&[all], ON, 80).contains("\x1b[32m●\x1b[0m"));
    }

    #[test]
    fn providers_table_fits_every_width() {
        let mut long = AvailRow::new("codex", ProviderStatus::Ready);
        long.plan = Some("pro".into());
        long.models = Some((0..30).map(|i| format!("model-number-{i}")).collect());
        long.verified = false;
        long.cooldown = Some("3h 20m".into());
        let mut all = rows();
        all.push(long);
        for w in [0, 10, MIN_WIDTH, 25, 30, 40, 60, 80, 120] {
            for style in [ON, OFF] {
                for l in providers_table(&all, style, w).split('\n') {
                    assert!(display_width(l) <= clamp_width(w), "{l:?} at {w}");
                }
            }
        }
        let s = providers_table(&all[5..], OFF, 60);
        assert!(s.contains("…") && s.ends_with("(unverified) · cooling 3h 20m"), "{s}");
    }

    // --------------------------------------------------------------- doctor

    fn item(sev: Severity, title: &str, detail: &str, fix: &[&str]) -> DoctorItem {
        DoctorItem { severity: sev, title: title.into(), detail: detail.into(), fix: fix.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn doctor_report_layout() {
        let items = vec![
            item(Severity::Fail, "codex is not logged in", "The probe could not see a login.", &["codex login", "/providers refresh"]),
            item(Severity::Warn, "No Jev key", "", &[]),
            item(Severity::Info, "Orchestration is off", "", &["/orchestrate on"]),
            item(Severity::Ok, "claude ready", "", &[]),
        ];
        assert_eq!(
            doctor_report(&items, OFF, 80),
            "✗ codex is not logged in\n    The probe could not see a login.\n    $ codex login\n    /providers refresh\n\n⚠ No Jev key\nⓘ Orchestration is off\n    /orchestrate on\n\n✓ claude ready"
        );
        assert_eq!(doctor_report(&[], OFF, 80), "");
        let styled = doctor_report(&items, ON, 80);
        assert!(styled.contains("\x1b[31m✗\x1b[0m") && styled.contains("\x1b[33m⚠\x1b[0m"));
        assert!(styled.contains("\x1b[36mⓘ\x1b[0m") && styled.contains("\x1b[32m✓\x1b[0m"));
        assert!(styled.contains("\x1b[36m$ codex login\x1b[0m"));
        assert!(styled.contains("\x1b[1mcodex is not logged in\x1b[0m"));
        assert_eq!(strip_ansi(&styled), doctor_report(&items, OFF, 80));
    }

    #[test]
    fn doctor_report_wraps_to_width() {
        let detail = "This is a long explanation of what went wrong with the setup, ".repeat(4);
        let items = vec![
            item(Severity::Fail, &"a very long title ".repeat(10), &detail, &["a-very-long-command --with --many --flags --that --do --not --fit --anywhere"]),
            item(Severity::Warn, "日本語のタイトル", "詳細な説明がここに入ります。とても長い説明です。", &[]),
        ];
        for w in [MIN_WIDTH, 30, 50, 80] {
            for style in [ON, OFF] {
                for l in doctor_report(&items, style, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w}");
                }
            }
        }
    }

    #[test]
    fn doctor_summaries() {
        let ok = item(Severity::Ok, "x", "", &[]);
        let warn = item(Severity::Warn, "x", "", &[]);
        let fail = item(Severity::Fail, "x", "", &[]);
        assert_eq!(doctor_summary(&[ok.clone()], OFF), "All good");
        assert_eq!(doctor_summary(&[ok.clone(), warn.clone()], OFF), "1 warning");
        assert_eq!(doctor_summary(&[fail.clone(), fail.clone()], OFF), "2 problems");
        assert_eq!(doctor_summary(&[fail, warn.clone(), warn], OFF), "1 problem · 2 warnings");
        assert_eq!(doctor_summary(&[], ON), "\x1b[32mAll good\x1b[0m");
    }

    // --------------------------------------------------------------- answer

    #[test]
    fn answer_text_is_untouched_without_style() {
        for t in [
            "# Title\n**bold** and `code`\n```rust\nfn main() {}\n```\n",
            "",
            "unclosed ```\n\x1b[31mraw\x1b[0m escapes stay in logs",
            "日本語 🚀 e\u{301}",
            &"word ".repeat(200),
        ] {
            assert_eq!(answer_text(t, OFF, 20), t);
        }
    }

    #[test]
    fn answer_headings_bold_and_code() {
        assert_eq!(answer_text("# Title", ON, 80), "\x1b[1mTitle\x1b[0m");
        assert_eq!(answer_text("### Deep ###", ON, 80), "\x1b[1mDeep\x1b[0m");
        assert_eq!(answer_text("#nospace", ON, 80), "#nospace");
        assert_eq!(answer_text("#", ON, 80), "#");
        assert_eq!(answer_text("####### seven", ON, 80), "####### seven");
        assert_eq!(answer_text("use **this** now", ON, 80), "use \x1b[1mthis\x1b[0m now");
        assert_eq!(answer_text("run `cargo test` now", ON, 80), "run \x1b[36mcargo\x1b[0m \x1b[36mtest\x1b[0m now");
        assert_eq!(answer_text("**bold**, then `x`.", ON, 80), "\x1b[1mbold\x1b[0m, then \x1b[36mx\x1b[0m.");
        assert_eq!(answer_text("**bold with `code`**", ON, 80), "\x1b[1mbold\x1b[0m \x1b[1mwith\x1b[0m \x1b[36mcode\x1b[0m");
        // unmatched markers are literal
        assert_eq!(answer_text("a ** b", ON, 80), "a ** b");
        assert_eq!(answer_text("a ` b", ON, 80), "a ` b");
        assert_eq!(answer_text("**unclosed bold", ON, 80), "**unclosed bold");
        assert_eq!(answer_text("2 ** 3 ** 4", ON, 80), "2 ** 3 ** 4");
        assert_eq!(answer_text("``a`b``", ON, 80), "\x1b[36ma`b\x1b[0m");
        assert_eq!(answer_text("****", ON, 80), "****");
    }

    #[test]
    fn answer_fenced_code() {
        let t = "before\n```rust\nfn main() {\n\n    let x = 1;\n}\n```\nafter";
        assert_eq!(
            answer_text(t, ON, 80),
            "before\n  \x1b[2mrust\x1b[0m\n    \x1b[2mfn main() {\x1b[0m\n\n    \x1b[2m    let x = 1;\x1b[0m\n    \x1b[2m}\x1b[0m\nafter"
        );
        // no language, tilde fence, longer closing fence
        assert_eq!(answer_text("~~~\ncode\n~~~~", ON, 80), "    \x1b[2mcode\x1b[0m");
        // markdown inside code is not interpreted and not wrapped
        let long = format!("```\n# not a heading **x** {}\n```", "word ".repeat(40));
        let out = answer_text(&long, ON, 30);
        assert!(strip_ansi(&out).contains("# not a heading **x**"));
        assert_eq!(out.matches('\n').count(), 0, "one code line, not wrapped");
        // a fence shorter than the opening one does not close it
        assert_eq!(answer_text("````\n```\nstill code\n````\nprose", ON, 80), "    \x1b[2m```\x1b[0m\n    \x1b[2mstill code\x1b[0m\nprose");
        // a closing fence cannot carry an info string
        assert_eq!(answer_text("```\n```rust\n```", ON, 80), "    \x1b[2m```rust\x1b[0m");
        // triple backticks inline are not a fence
        assert_eq!(answer_text("use ```x``` here", ON, 80), "use \x1b[36mx\x1b[0m here");
    }

    #[test]
    fn answer_unclosed_fence_swallows_the_rest_as_code() {
        let out = answer_text("```py\nprint(1)\n# still code\n**not bold**", ON, 80);
        assert_eq!(
            out,
            "  \x1b[2mpy\x1b[0m\n    \x1b[2mprint(1)\x1b[0m\n    \x1b[2m# still code\x1b[0m\n    \x1b[2m**not bold**\x1b[0m"
        );
        assert_eq!(answer_text("```", ON, 80), "");
    }

    #[test]
    fn answer_lists_keep_markers_and_hang() {
        assert_eq!(answer_text("- one\n* two\n+ three", ON, 80), "- one\n* two\n+ three");
        assert_eq!(answer_text("1. first\n12) second", ON, 80), "1. first\n12) second");
        assert_eq!(answer_text("  - nested **b**", ON, 80), "  - nested \x1b[1mb\x1b[0m");
        let out = answer_text("- alpha beta gamma delta epsilon zeta", ON, 20);
        assert_eq!(out, "- alpha beta gamma\n  delta epsilon zeta");
        let out = answer_text("10. alpha beta gamma delta", ON, 20);
        assert_eq!(out, "10. alpha beta gamma\n    delta");
        // a bold line starting with ** is not a bullet
        assert_eq!(answer_text("**Note** here", ON, 80), "\x1b[1mNote\x1b[0m here");
        // "-5 degrees" is prose
        assert_eq!(answer_text("-5 degrees", ON, 80), "-5 degrees");
        // quotes keep their marker
        assert_eq!(answer_text("> quoted text", ON, 80), "> quoted text");
    }

    #[test]
    fn answer_wraps_prose_not_urls_tables_or_blank_lines() {
        let url = "https://example.com/some/really/long/path/that/never/fits/on/one/line";
        let out = answer_text(&format!("see {url} for more details on this"), ON, 30);
        assert!(out.contains(url));
        assert!(out.split('\n').any(|l| l == url));
        let table = "| a | b |\n|---|---|\n| a very long cell that would wrap if it were prose | x |";
        assert_eq!(answer_text(table, ON, 20), table);
        assert_eq!(answer_text("a\n\n\nb\n", ON, 80), "a\n\n\nb\n");
        assert_eq!(answer_text("one two three four five six", ON, 20), "one two three four\nfive six");
        assert_eq!(answer_text("a\r\nb\r\n", ON, 80), "a\nb\n");
        // two-line paragraphs stay two lines (the model chose those breaks)
        assert_eq!(answer_text("first line\nsecond line", ON, 80), "first line\nsecond line");
        // leading indentation of prose is kept, but never leaves no room
        assert_eq!(answer_text("  indented text", ON, 80), "  indented text");
        let deep = format!("{}text that is long enough to wrap around", " ".repeat(30));
        for l in answer_text(&deep, ON, 20).split('\n') {
            assert!(display_width(l) <= 20, "{l:?}");
        }
    }

    #[test]
    fn answer_styled_spans_survive_wrapping() {
        let out = answer_text("**alpha beta gamma delta** epsilon zeta", ON, 20);
        assert_eq!(strip_ansi(&out), "alpha beta gamma\ndelta epsilon zeta");
        // each word opens and closes its own bold, so a wrap never leaks a style
        assert_eq!(out.split('\n').next().unwrap(), "\x1b[1malpha\x1b[0m \x1b[1mbeta\x1b[0m \x1b[1mgamma\x1b[0m");
    }

    #[test]
    fn answer_prose_never_exceeds_the_width() {
        let samples = [
            "The quick brown fox jumps over the lazy dog and keeps running through the forest.",
            "日本語のテキストはスペースがないのでとても長い一つの単語として扱われます。これは長い。",
            "🚀🚀🚀 launch 🚀🚀 sequence 🚀 initiated 🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀",
            "supercalifragilisticexpialidocious_and_then_some_more_characters_without_any_space",
            "**bold supercalifragilisticexpialidocious_and_then_some_more_characters** `inlinecode_that_is_quite_long_indeed`",
            "- a list item with `code` and **bold** and a verylongwordthatdoesnotfitanywhereatall_really",
            "e\u{301}e\u{301}e\u{301} combining marks e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
            "# A heading that is long enough that it has to wrap onto another row of text",
            "> a quote that is long enough that it has to wrap onto another row of text",
        ];
        for t in samples {
            for w in [MIN_WIDTH, 21, 25, 33, 40, 80] {
                for l in answer_text(t, ON, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w} for {t:?}");
                }
            }
        }
    }

    #[test]
    fn answer_text_survives_weird_input() {
        let weird = [
            "",
            "\n\n\n",
            "```",
            "``````",
            "`",
            "``",
            "`a",
            "**",
            "*",
            "* ",
            "- ",
            "1.",
            "1. ",
            "#",
            "# ",
            "| ",
            "\t\t- tabbed",
            "\x1b[31mred\x1b[0m text",
            "\x1b",
            "\u{0}\u{7}\u{8}",
            "🚀`🚀**🚀`**",
            "**`**`**",
            &"`".repeat(500),
            &"*".repeat(501),
            &"a".repeat(10_000),
            &"- ".repeat(1_000),
            "e\u{301}",
            "\u{200d}\u{200d}",
            "```rust\n```rust\n```\n```",
            "> > > > >",
            "99999. not a list",
        ];
        for t in weird {
            for w in [0, 1, 20, 80] {
                let out = answer_text(t, ON, w);
                // model text can never drive the terminal: only our own SGR sequences remain
                let mut rest = out.as_str();
                while let Some(i) = rest.find('\x1b') {
                    let after = &rest[i..];
                    let end = after.find('m').expect("only SGR sequences");
                    assert!(after[1..end].chars().all(|c| c == '[' || c.is_ascii_digit() || c == ';'), "{out:?}");
                    rest = &after[end + 1..];
                }
                let _ = answer_text(t, OFF, w);
            }
        }
    }

    #[test]
    fn answer_text_strips_model_escapes() {
        let out = answer_text("hello \x1b[2Jworld \x1b]0;title\x07done", ON, 80);
        assert_eq!(out, "hello world done");
        assert_eq!(answer_text("```\n\x1b[31mred\n```", ON, 80), "    \x1b[2mred\x1b[0m");
    }

    // ---------------------------------------------------------- status line

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

    // ------------------------------------------------------------ completion

    fn ctx() -> CompleteCtx {
        CompleteCtx {
            ready_providers: vec!["claude".into(), "codex".into()],
            models: vec!["claude:claude-sonnet-5-5".into(), "codex:gpt-6.1-sol".into(), "codex:gpt-6-luna".into()],
            changed_files: vec!["src/a.rs".into(), "src/b.rs".into(), "README.md".into(), "has space.txt".into()],
        }
    }

    fn replacements(line: &str) -> Vec<String> {
        complete(line, &ctx()).into_iter().map(|c| c.replacement).collect()
    }

    #[test]
    fn completes_slash_commands() {
        assert_eq!(replacements("/"), vec!["/diff", "/doctor", "/help", "/model", "/new", "/orchestrate", "/provider", "/providers", "/quit", "/resume", "/status", "/undo"]);
        assert_eq!(replacements("/mo"), vec!["/model"]);
        assert_eq!(replacements("/prov"), vec!["/provider", "/providers"]);
        assert_eq!(replacements("/MO"), vec!["/model"]);
        assert_eq!(replacements("/nope"), Vec::<String>::new());
        // the menu line names the command and says what it does
        let c = &complete("/mo", &ctx())[0];
        assert!(c.display.starts_with("/model") && c.display.contains("Pin a model"), "{}", c.display);
        // every command is a candidate for its own full name
        for cmd in &COMMANDS {
            assert!(replacements(cmd.name).contains(&cmd.name.to_string()));
        }
    }

    #[test]
    fn display_columns_align() {
        let cs = complete("/", &ctx());
        let col = cs[0].display.find(|c: char| c.is_uppercase()).unwrap();
        assert!(cs.iter().all(|c| c.display.find(|ch: char| ch.is_uppercase()) == Some(col)));
    }

    #[test]
    fn completes_arguments() {
        assert_eq!(
            replacements("/model "),
            vec!["auto", "claude", "codex", "claude:claude-sonnet-5-5", "codex:gpt-6.1-sol", "codex:gpt-6-luna", "sonnet", "opus", "haiku"]
        );
        assert_eq!(replacements("/model c"), vec!["claude", "codex", "claude:claude-sonnet-5-5", "codex:gpt-6.1-sol", "codex:gpt-6-luna"]);
        assert_eq!(replacements("/model codex:"), vec!["codex:gpt-6.1-sol", "codex:gpt-6-luna"]);
        assert_eq!(replacements("/model SO"), vec!["sonnet"]);
        assert_eq!(replacements("/model a"), vec!["auto"]);
        assert_eq!(replacements("/provider "), vec!["auto", "off"]);
        assert_eq!(replacements("/provider o"), vec!["off"]);
        assert_eq!(replacements("/orchestrate "), vec!["on", "off"]);
        assert_eq!(replacements("/orchestrate of"), vec!["off"]);
        assert_eq!(replacements("/providers r"), vec!["refresh"]);
        assert_eq!(replacements("/providers "), vec!["refresh"]);
        assert_eq!(replacements("/diff "), vec!["src/a.rs", "src/b.rs", "README.md"]);
        assert_eq!(replacements("/diff src/b"), vec!["src/b.rs"]);
        // a second argument, commands without arguments, prose and unknown commands complete nothing
        for none in ["/model auto ", "/model auto x", "/new ", "/help x", "fix the bug", "fix /model ", "", " /model ", "/nope "] {
            assert_eq!(replacements(none), Vec::<String>::new(), "{none:?}");
        }
        // duplicates collapse (a provider named like an alias)
        let dup = CompleteCtx { ready_providers: vec!["claude".into(), "claude".into()], ..Default::default() };
        let n = complete("/model cl", &dup).len();
        assert_eq!(n, 1);
        // an empty context still offers the fixed choices
        let empty: Vec<String> = complete("/model ", &CompleteCtx::default()).into_iter().map(|c| c.replacement).collect();
        assert_eq!(empty, vec!["auto", "sonnet", "opus", "haiku"]);
    }

    #[test]
    fn completion_start_is_the_word_start() {
        for (line, at) in [("", 0), ("/mo", 0), ("/model ", 7), ("/model co", 7), ("/model  co", 8), ("/diff src/a", 6), ("/model\tx", 7), ("日本 語", 7)] {
            assert_eq!(completion_start(line), at, "{line:?}");
        }
        // applying a candidate yields the full line
        let line = "/model co";
        let c = &complete(line, &ctx())[0];
        assert_eq!(format!("{}{}", &line[..completion_start(line)], c.replacement), "/model codex");
    }

    #[test]
    fn hints() {
        for (line, want) in [
            ("/model ", Some("auto|codex|claude:...")),
            ("/model", Some(" auto|codex|claude:...")),
            ("/provider ", Some("auto|off")),
            ("/orchestrate ", Some("on|off")),
            ("/providers ", Some("[refresh]")),
            ("/diff ", Some("[file]")),
            ("/mod", Some("el")),
            ("/prov", Some("ider")),
            ("/provider", Some(" auto|off")),
            ("/", Some("diff")),
            ("/new", None),
            ("/new ", None),
            ("/model co", None),
            ("/model auto ", None),
            ("/nope", None),
            ("hello", None),
            ("", None),
        ] {
            assert_eq!(hint(line).as_deref(), want, "{line:?}");
        }
        for line in ["/model ", "/mod", "/"] {
            no_esc(&hint(line).unwrap());
        }
    }

    // ------------------------------------------------------- global invariants

    #[test]
    fn a_disabled_style_never_emits_escapes() {
        let all_rows = rows();
        let kinds = all_kinds();
        for kind in &kinds {
            no_esc(&failover_note("claude", "opus", kind, Some(("codex", "g")), true, OFF));
            no_esc(&error_block(kind, "message", &next_steps_for(kind, "claude"), OFF));
        }
        no_esc(&route_line(&RouteFacts { provider: "codex".into(), model: "m".into(), reason: "task: a (0.2) → b; first turn".into(), ..Default::default() }, OFF));
        no_esc(&no_route_message("security", &all_rows, OFF));
        no_esc(&turn_summary(&facts(), OFF));
        no_esc(&changes_block(&[fc('M', "a", 1, 1)], OFF, 40));
        no_esc(&diff_colorize("+a\n-b\n@@ x\n", OFF));
        no_esc(&tool_line("Edit a", Path::new("/p"), OFF, 40));
        no_esc(&help_text(OFF));
        no_esc(&status_text(&StatusFacts::default(), OFF));
        no_esc(&providers_table(&all_rows, OFF, 80));
        no_esc(&doctor_report(&[item(Severity::Fail, "t", "d", &["c"])], OFF, 80));
        no_esc(&doctor_summary(&[item(Severity::Fail, "t", "d", &["c"])], OFF));
        no_esc(&status_line("m", Duration::from_secs(1), Some("a"), 40, OFF));
    }

    #[test]
    fn provider_glyphs_fit_on_one_line_and_use_the_table_glyphs() {
        let line = provider_glyphs(&rows(), OFF, 80);
        assert_eq!(line, "◐ claude  ○ codex  ✕ gemini  ○ deepseek  ? grok");
        assert!(display_width(&provider_glyphs(&rows(), OFF, 20)) <= 20);
        assert_eq!(strip_ansi(&provider_glyphs(&rows(), ON, 80)), line);
        assert!(provider_glyphs(&rows(), ON, 80).contains("\x1b["), "coloured when styled");
    }

    #[test]
    fn the_command_table_gives_names_and_usage_to_parsing() {
        let names = command_names();
        assert!(names.contains(&"/model") && names.contains(&"/help") && names.contains(&"/undo"));
        assert_eq!(command_usage("/diff").as_deref(), Some("/diff [file]"));
        assert_eq!(command_usage("/new").as_deref(), Some("/new"));
        assert_eq!(command_usage("/nope"), None);
    }

    #[test]
    fn a_problem_block_has_a_title_details_and_steps() {
        let s = problem_block("Title", &["why".to_string()], &["Do `x`".to_string()], OFF);
        assert_eq!(s, "✗ Title\n  why\n  Try:\n    • Do `x`");
    }

    #[test]
    fn styling_only_adds_escapes_never_words() {
        let all_rows = rows();
        let pairs: Vec<(String, String)> = vec![
            (
                no_route_message("security", &all_rows, ON),
                no_route_message("security", &all_rows, OFF),
            ),
            (
                error_block(&ErrorKind::Auth, "bad\nthing", &["Do `x`".to_string()], ON),
                error_block(&ErrorKind::Auth, "bad\nthing", &["Do `x`".to_string()], OFF),
            ),
            (changes_block(&[fc('M', "a", 1, 1)], ON, 40), changes_block(&[fc('M', "a", 1, 1)], OFF, 40)),
            (tool_line("Edit a", Path::new("/p"), ON, 40), tool_line("Edit a", Path::new("/p"), OFF, 40)),
            (help_text(ON), help_text(OFF)),
            (doctor_summary(&[item(Severity::Warn, "t", "", &[])], ON), doctor_summary(&[item(Severity::Warn, "t", "", &[])], OFF)),
            (status_line("m", Duration::from_secs(3), Some("a"), 40, ON), status_line("m", Duration::from_secs(3), Some("a"), 40, OFF)),
        ];
        for (styled, plain) in pairs {
            assert_eq!(strip_ansi(&styled), plain);
        }
    }
}
