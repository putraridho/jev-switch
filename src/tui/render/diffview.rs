//! A diff for reading: line numbers, intra-line (word) highlighting, syntax colour in
//! the hunks, folding of long files, and a side-by-side layout on wide terminals.
//!
//! Input is the parsed diff ([`crate::changes::domain::diff_parse`]); output is plain strings with no
//! trailing newline. A disabled [`Style`] produces zero escape bytes, and no line is
//! wider than the width asked for.

use crate::changes::domain::diff_parse::{FileDiff, Hunk, LineKind};

use crate::tui::render::highlight::{highlight_line, Lang, LineState};
use crate::tui::render::style::Style;
use crate::tui::render::width::{clamp_width, pad_to_width, truncate_to_width};

/// Terminals at least this wide get the side-by-side layout when the layout is `Auto`.
pub const SPLIT_MIN_WIDTH: usize = 110;

/// Body lines of one file shown before the rest is folded, unless told otherwise.
pub const FOLD_AFTER: usize = 60;

/// Two lines count as "the same line edited" for word highlighting when at least this
/// share (percent) of the longer one is unchanged; otherwise marking words is noise.
const MIN_SIMILARITY_PERCENT: usize = 40;

/// Most words per line the word diff will compare (it is quadratic).
const MAX_WORD_TOKENS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Side by side when the width allows it, else unified.
    Auto,
    Unified,
    Split,
}

/// How much of each file to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// Everything.
    Open,
    /// This many body lines per file, then a `⋯ N more lines` row.
    After(usize),
    /// Only the file rows: every diff collapsed.
    Collapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOpts {
    pub width: usize,
    pub layout: Layout,
    pub fold: Fold,
}

impl DiffOpts {
    pub fn new(width: usize) -> Self {
        Self { width, layout: Layout::Auto, fold: Fold::After(FOLD_AFTER) }
    }

    fn split(&self) -> bool {
        match self.layout {
            Layout::Unified => false,
            Layout::Split => true,
            Layout::Auto => clamp_width(self.width) >= SPLIT_MIN_WIDTH,
        }
    }
}

// ------------------------------------------------------------------ word diff

/// A piece of a line and whether it differs from the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub changed: bool,
}

/// Words, runs of blanks, and single punctuation characters.
fn tokens(line: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| if c.is_alphanumeric() || c == '_' { Class::Word } else if c.is_whitespace() { Class::Space } else { Class::Other };
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<Class> = None;
    for (i, c) in line.char_indices() {
        let k = class(c);
        if prev.is_some_and(|p| p != k || k == Class::Other) {
            out.push(&line[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

/// Longest-common-subsequence table suffix lengths: `t[i][j]` for `a[i..]`, `b[j..]`.
fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<u16>> {
    let mut t = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
        }
    }
    t
}

fn push_piece(pieces: &mut Vec<Piece>, text: &str, changed: bool) {
    match pieces.last_mut() {
        Some(last) if last.changed == changed => last.text.push_str(text),
        _ => pieces.push(Piece { text: text.to_string(), changed }),
    }
}

/// The old and the new line cut into pieces, the pieces that are not in both marked
/// `changed`. `None` when the lines are too long or too different for the marks to help.
pub fn word_diff(old: &str, new: &str) -> Option<(Vec<Piece>, Vec<Piece>)> {
    let (a, b) = (tokens(old), tokens(new));
    if a.len() > MAX_WORD_TOKENS || b.len() > MAX_WORD_TOKENS {
        return None;
    }
    let t = lcs_table(&a, &b);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut i, mut j, mut same) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            same += a[i].chars().count();
            push_piece(&mut left, a[i], false);
            push_piece(&mut right, b[j], false);
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            push_piece(&mut left, a[i], true);
            i += 1;
        } else {
            push_piece(&mut right, b[j], true);
            j += 1;
        }
    }
    for tok in &a[i..] {
        push_piece(&mut left, tok, true);
    }
    for tok in &b[j..] {
        push_piece(&mut right, tok, true);
    }
    let longest = old.chars().count().max(new.chars().count()).max(1);
    (same * 100 / longest >= MIN_SIMILARITY_PERCENT && same > 0).then_some((left, right))
}

// ------------------------------------------------------------------ rows

/// One line of one side of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    number: u32,
    kind: LineKind,
    text: String,
    /// Set when this line pairs with an edited line on the other side.
    pieces: Option<Vec<Piece>>,
}

/// A stretch of a hunk: unchanged lines, or a change (lines removed, lines added).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Same { old: u32, new: u32, text: String },
    Change { removed: Vec<Cell>, added: Vec<Cell> },
}

fn expand_tabs(s: &str) -> String {
    s.replace('\t', "    ")
}

fn blocks_of(hunk: &Hunk) -> Vec<Block> {
    let mut blocks = Vec::new();
    let (mut old, mut new) = (hunk.old_start, hunk.new_start);
    let (mut removed, mut added): (Vec<Cell>, Vec<Cell>) = (Vec::new(), Vec::new());
    let flush = |blocks: &mut Vec<Block>, removed: &mut Vec<Cell>, added: &mut Vec<Cell>| {
        if removed.is_empty() && added.is_empty() {
            return;
        }
        for (r, a) in removed.iter_mut().zip(added.iter_mut()) {
            if let Some((left, right)) = word_diff(&r.text, &a.text) {
                r.pieces = Some(left);
                a.pieces = Some(right);
            }
        }
        blocks.push(Block::Change { removed: std::mem::take(removed), added: std::mem::take(added) });
    };
    for line in &hunk.lines {
        let text = expand_tabs(&line.text);
        match line.kind {
            LineKind::Context => {
                flush(&mut blocks, &mut removed, &mut added);
                blocks.push(Block::Same { old, new, text });
                old += 1;
                new += 1;
            }
            LineKind::Removed => {
                removed.push(Cell { number: old, kind: LineKind::Removed, text, pieces: None });
                old += 1;
            }
            LineKind::Added => {
                added.push(Cell { number: new, kind: LineKind::Added, text, pieces: None });
                new += 1;
            }
            LineKind::NoNewline => {}
        }
    }
    flush(&mut blocks, &mut removed, &mut added);
    blocks
}

// ------------------------------------------------------------------ painting

fn lang_of(path: &str) -> Option<Lang> {
    path.rsplit_once('.').and_then(|(_, ext)| Lang::from_info(ext))
}

/// The text of a cell: edited lines in their side's colour with the changed words
/// reversed; everything else syntax-coloured when the language is known.
fn paint_text(cell_text: &str, pieces: Option<&[Piece]>, kind: LineKind, lang: Option<Lang>, state: &mut LineState, style: Style) -> String {
    if !style.enabled {
        return cell_text.to_string();
    }
    if let Some(pieces) = pieces {
        let side = |s: &str| if kind == LineKind::Removed { style.red(s) } else { style.green(s) };
        return pieces.iter().map(|p| if p.changed { style.reverse(&side(&p.text)) } else { side(&p.text) }).collect();
    }
    match lang {
        Some(l) => highlight_line(l, cell_text, state, style),
        None => cell_text.to_string(),
    }
}

fn marker(kind: LineKind, style: Style) -> String {
    match kind {
        LineKind::Added => style.bold(&style.green("+")),
        LineKind::Removed => style.bold(&style.red("-")),
        _ => " ".to_string(),
    }
}

fn num(n: Option<u32>, w: usize, style: Style) -> String {
    style.dim(&format!("{:>w$}", n.map_or(String::new(), |n| n.to_string())))
}

fn digits(n: u32) -> usize {
    n.max(1).to_string().len()
}

/// Width of the line-number column for a file: the widest number any hunk reaches.
fn number_width(file: &FileDiff) -> usize {
    let top = file.hunks.iter().map(|h| (h.old_start + h.lines.len() as u32).max(h.new_start + h.lines.len() as u32)).max().unwrap_or(1);
    digits(top)
}

/// One diff row in the unified layout: `old new ± text`.
fn unified_row(old: Option<u32>, new: Option<u32>, kind: LineKind, text: String, w: usize, style: Style, width: usize) -> String {
    let line = format!("  {} {} {} {}", num(old, w, style), num(new, w, style), marker(kind, style), text);
    truncate_to_width(line.trim_end(), width)
}

/// The rows of one hunk, unified.
fn unified_rows(hunk: &Hunk, lang: Option<Lang>, w: usize, style: Style, width: usize) -> Vec<String> {
    let mut state = LineState::default();
    let mut rows = Vec::new();
    for block in blocks_of(hunk) {
        match block {
            Block::Same { old, new, text } => {
                let t = paint_text(&text, None, LineKind::Context, lang, &mut state, style);
                rows.push(unified_row(Some(old), Some(new), LineKind::Context, t, w, style, width));
            }
            Block::Change { removed, added } => {
                for c in &removed {
                    let t = paint_text(&c.text, c.pieces.as_deref(), c.kind, lang, &mut state, style);
                    rows.push(unified_row(Some(c.number), None, c.kind, t, w, style, width));
                }
                for c in &added {
                    let t = paint_text(&c.text, c.pieces.as_deref(), c.kind, lang, &mut state, style);
                    rows.push(unified_row(None, Some(c.number), c.kind, t, w, style, width));
                }
            }
        }
    }
    rows
}

/// One half of a side-by-side row, exactly `half` columns wide.
fn half_cell(number: Option<u32>, kind: LineKind, text: &str, w: usize, half: usize, style: Style) -> String {
    let body = if text.is_empty() && number.is_none() { String::new() } else { format!("{} {} {}", num(number, w, style), marker(kind, style), text) };
    pad_to_width(&truncate_to_width(&body, half), half)
}

/// The rows of one hunk, side by side: old on the left, new on the right.
fn split_rows(hunk: &Hunk, lang: Option<Lang>, w: usize, style: Style, width: usize) -> Vec<String> {
    let half = (width.saturating_sub(2 + 3) / 2).max(1);
    let sep = style.dim(" │ ");
    let mut state = LineState::default();
    let mut rows = Vec::new();
    let mut row = |l: String, r: String| rows.push(format!("  {l}{sep}{r}").trim_end().to_string());
    for block in blocks_of(hunk) {
        match block {
            Block::Same { old, new, text } => {
                let t = paint_text(&text, None, LineKind::Context, lang, &mut state, style);
                row(half_cell(Some(old), LineKind::Context, &t, w, half, style), half_cell(Some(new), LineKind::Context, &t, w, half, style));
            }
            Block::Change { removed, added } => {
                for i in 0..removed.len().max(added.len()) {
                    let side = |c: Option<&Cell>, state: &mut LineState| match c {
                        Some(c) => half_cell(Some(c.number), c.kind, &paint_text(&c.text, c.pieces.as_deref(), c.kind, lang, state, style), w, half, style),
                        None => half_cell(None, LineKind::Context, "", w, half, style),
                    };
                    let (l, r) = (side(removed.get(i), &mut state), side(added.get(i), &mut state));
                    row(l, r);
                }
            }
        }
    }
    rows
}

/// The `- old` / `+ new` lines of an edit approval card, coloured: removed red, added
/// green, and when a run of removed lines is followed by as many added ones the words that
/// differ are reversed. One entry per input line, `None` for a line that is not a change
/// line (the path row, `edit 2:`) or when colour is off. A painted entry has the text of
/// its line exactly; only escapes are added.
pub(super) fn paint_edit_lines(lines: &[String], style: Style) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; lines.len()];
    if !style.enabled {
        return out;
    }
    fn side(l: &str) -> Option<(LineKind, &str)> {
        l.strip_prefix("- ").map(|t| (LineKind::Removed, t)).or_else(|| l.strip_prefix("+ ").map(|t| (LineKind::Added, t)))
    }
    let paint = |kind: LineKind, marker: &str, pieces: Option<&[Piece]>, text: &str| {
        let color = |s: &str| if kind == LineKind::Removed { style.red(s) } else { style.green(s) };
        let body = match pieces {
            Some(p) => p.iter().map(|p| if p.changed { style.reverse(&color(&p.text)) } else { color(&p.text) }).collect(),
            None => color(text),
        };
        format!("{}{body}", color(marker))
    };
    let mut i = 0;
    while i < lines.len() {
        let run = |from: usize, kind: LineKind| lines[from..].iter().take_while(|l| side(l).is_some_and(|(k, _)| k == kind)).count();
        let removed = run(i, LineKind::Removed);
        let added = run(i + removed, LineKind::Added);
        if removed + added == 0 {
            i += 1;
            continue;
        }
        let paired = removed == added;
        for k in 0..removed + added {
            let Some((kind, text)) = side(&lines[i + k]) else { continue };
            let marker = if kind == LineKind::Removed { "- " } else { "+ " };
            let other = if paired { if k < removed { side(&lines[i + k + removed]) } else { side(&lines[i + k - removed]) } } else { None };
            let pieces = other.and_then(|(_, o)| if kind == LineKind::Removed { word_diff(text, o).map(|p| p.0) } else { word_diff(o, text).map(|p| p.1) });
            out[i + k] = Some(paint(kind, marker, pieces.as_deref(), text));
        }
        i += removed + added;
    }
    out
}

fn counts(added: usize, removed: usize, style: Style) -> String {
    format!("{} {}", style.green(&format!("+{added}")), style.red(&format!("−{removed}")))
}

fn file_row(file: &FileDiff, style: Style, width: usize) -> String {
    truncate_to_width(&format!("{} {}  {}", style.cyan("▸"), style.bold(&file.path), counts(file.added(), file.removed(), style)), width)
}

fn fold_row(hidden: usize, style: Style) -> String {
    let noun = if hidden == 1 { "line" } else { "lines" };
    style.dim(&format!("  ⋯ {hidden} more {noun} folded (/diff --full to show)"))
}

/// Rows for one hunk header: cyan, cut to the width.
fn hunk_row(hunk: &Hunk, style: Style, width: usize) -> String {
    truncate_to_width(&format!("  {}", style.cyan(&hunk.header)), width)
}

fn hunk_rows(file: &FileDiff, hunk: &Hunk, opts: &DiffOpts, style: Style) -> Vec<String> {
    let width = clamp_width(opts.width);
    let (lang, w) = (lang_of(&file.path), number_width(file));
    let mut rows = vec![hunk_row(hunk, style, width)];
    rows.extend(if opts.split() { split_rows(hunk, lang, w, style, width) } else { unified_rows(hunk, lang, w, style, width) });
    rows
}

/// One file: its row, then its hunks, folded as `opts.fold` says.
fn render_file(file: &FileDiff, opts: &DiffOpts, style: Style) -> Vec<String> {
    let width = clamp_width(opts.width);
    let mut out = vec![file_row(file, style, width)];
    if file.hunks.is_empty() {
        out.push(style.dim("  (no text changes: binary file, rename or mode change)"));
        return out;
    }
    let all: Vec<Vec<String>> = file.hunks.iter().map(|h| hunk_rows(file, h, opts, style)).collect();
    let total: usize = all.iter().map(Vec::len).sum();
    let limit = match opts.fold {
        Fold::Open => total,
        Fold::After(n) => n,
        Fold::Collapsed => 0,
    };
    if limit == 0 {
        out.push(fold_row(total, style));
        return out;
    }
    let mut shown = 0;
    for rows in all {
        for row in rows {
            if shown == limit {
                break;
            }
            out.push(row);
            shown += 1;
        }
    }
    if shown < total {
        out.push(fold_row(total - shown, style));
    }
    out
}

/// A whole diff, one block per file separated by a blank row.
pub fn render_diff(files: &[FileDiff], opts: &DiffOpts, style: Style) -> String {
    files.iter().map(|f| render_file(f, opts, style).join("\n")).collect::<Vec<_>>().join("\n\n")
}

/// One hunk of one file, with its file row: what a per-hunk review shows.
pub fn render_hunk(file: &FileDiff, index: usize, opts: &DiffOpts, style: Style) -> Option<String> {
    let hunk = file.hunks.get(index)?;
    let mut rows = vec![truncate_to_width(
        &format!("{} {}  {}", style.cyan("▸"), style.bold(&file.path), style.dim(&format!("hunk {} of {}", index + 1, file.hunks.len()))),
        clamp_width(opts.width),
    )];
    rows.extend(hunk_rows(file, hunk, opts, style));
    Some(rows.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::domain::diff_parse::parse;
    use crate::tui::render::strip_ansi;
    use crate::tui::render::test_facts::*;
    use crate::tui::render::width::display_width;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -9,4 +9,5 @@ fn main\n let a = 1;\n-let total = price * 3;\n+let total = price * quantity;\n+let extra = 0;\n done();\n";

    fn opts(width: usize, layout: Layout) -> DiffOpts {
        DiffOpts { width, layout, fold: Fold::Open }
    }

    #[test]
    fn word_diff_marks_only_the_changed_words() {
        let (l, r) = word_diff("let total = price * 3;", "let total = price * quantity;").unwrap();
        let changed = |p: &[Piece]| p.iter().filter(|p| p.changed).map(|p| p.text.clone()).collect::<Vec<_>>();
        assert_eq!(changed(&l), ["3"]);
        assert_eq!(changed(&r), ["quantity"]);
        assert_eq!(l.iter().map(|p| p.text.as_str()).collect::<String>(), "let total = price * 3;");
    }

    #[test]
    fn unrelated_lines_get_no_word_marks() {
        assert!(word_diff("alpha beta gamma", "zzz").is_none());
        assert!(word_diff(&"x ".repeat(300), "y").is_none(), "too long to compare");
    }

    #[test]
    fn unified_has_line_numbers_markers_and_text_without_colour() {
        let out = render_diff(&parse(DIFF), &opts(80, Layout::Unified), OFF);
        no_esc(&out);
        let l = lines(&out);
        assert_eq!(l[0], "▸ src/a.rs  +2 −1");
        assert_eq!(l[1], "  @@ -9,4 +9,5 @@ fn main");
        assert_eq!(l[2], "   9  9   let a = 1;");
        assert_eq!(l[3], "  10    - let total = price * 3;");
        assert_eq!(l[4], "     10 + let total = price * quantity;");
        assert_eq!(l[5], "     11 + let extra = 0;");
        assert_eq!(l[6], "  11 12   done();");
    }

    #[test]
    fn colour_adds_escapes_but_never_changes_the_text() {
        let plain = render_diff(&parse(DIFF), &opts(80, Layout::Unified), OFF);
        let coloured = render_diff(&parse(DIFF), &opts(80, Layout::Unified), ON);
        assert!(coloured.contains('\x1b'));
        assert_eq!(strip_ansi(&coloured), plain);
    }

    #[test]
    fn changed_words_are_reversed_and_context_is_syntax_coloured() {
        let out = render_diff(&parse(DIFF), &opts(80, Layout::Unified), ON);
        assert!(out.contains("\x1b[7m"), "reverse video marks the words that changed: {out:?}");
        let ctx = out.lines().find(|l| strip_ansi(l).contains("let a = 1;")).unwrap();
        assert!(ctx.contains("\x1b[") && ctx.matches('\x1b').count() > 6, "keywords and numbers coloured in {ctx:?}");
    }

    #[test]
    fn side_by_side_pairs_old_and_new_and_fits_the_width() {
        let out = render_diff(&parse(DIFF), &opts(100, Layout::Split), OFF);
        let l = lines(&out);
        let pair = l.iter().find(|l| l.contains("price * 3")).unwrap();
        assert!(pair.contains("price * 3;") && pair.contains("│") && pair.contains("price * quantity;"), "{pair}");
        let extra = l.iter().find(|l| l.contains("let extra")).unwrap();
        assert!(extra.find('│').unwrap() < extra.find("let extra").unwrap(), "an added line with no old side leaves the left empty: {extra}");
        assert!(l.iter().all(|l| display_width(l) <= 100), "{out}");
    }

    #[test]
    fn auto_layout_goes_side_by_side_only_when_wide() {
        let narrow = render_diff(&parse(DIFF), &DiffOpts { width: 80, layout: Layout::Auto, fold: Fold::Open }, OFF);
        let wide = render_diff(&parse(DIFF), &DiffOpts { width: 140, layout: Layout::Auto, fold: Fold::Open }, OFF);
        assert!(!narrow.contains('│') && wide.contains('│'));
    }

    #[test]
    fn long_files_fold_and_say_how_to_open_them() {
        let mut text = String::from("diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -0,0 +1,100 @@\n");
        for i in 0..100 {
            text.push_str(&format!("+line {i}\n"));
        }
        let files = parse(&text);
        let folded = render_diff(&files, &DiffOpts { width: 80, layout: Layout::Unified, fold: Fold::After(10) }, OFF);
        assert_eq!(folded.lines().count(), 1 + 10 + 1);
        assert!(folded.contains("⋯ 91 more lines folded (/diff --full to show)"), "{folded}");
        let collapsed = render_diff(&files, &DiffOpts { width: 80, layout: Layout::Unified, fold: Fold::Collapsed }, OFF);
        assert_eq!(collapsed.lines().count(), 2);
        assert_eq!(render_diff(&files, &opts(80, Layout::Unified), OFF).lines().count(), 1 + 1 + 100);
    }

    #[test]
    fn a_file_with_no_hunks_says_why() {
        let files = parse("diff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n");
        assert!(render_diff(&files, &opts(80, Layout::Unified), OFF).contains("no text changes"));
    }

    #[test]
    fn a_hunk_view_names_its_position_and_no_line_is_too_wide() {
        let files = parse(DIFF);
        let out = render_hunk(&files[0], 0, &opts(30, Layout::Unified), OFF).unwrap();
        assert!(out.starts_with("▸ src/a.rs  hunk 1 of 1"), "{out}");
        assert!(out.lines().all(|l| display_width(l) <= 30), "{out}");
        assert!(render_hunk(&files[0], 1, &opts(30, Layout::Unified), OFF).is_none());
    }

    #[test]
    fn tabs_and_wide_characters_stay_inside_the_width() {
        let text = "diff --git a/m.go b/m.go\n--- a/m.go\n+++ b/m.go\n@@ -1 +1 @@\n-\tx := \"日本語日本語日本語\"\n+\ty := \"日本語日本語日本語\"\n";
        for layout in [Layout::Unified, Layout::Split] {
            let out = render_diff(&parse(text), &opts(40, layout), OFF);
            assert!(out.lines().all(|l| display_width(l) <= 40), "{out}");
        }
    }
}
