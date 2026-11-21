//! A diff for reading: line numbers, intra-line (word) highlighting, syntax colour in
//! the hunks, folding of long files, and a side-by-side layout on wide terminals.
//!
//! Input is the parsed diff ([`jev_changes::domain::diff_parse`]); output is plain strings with no
//! trailing newline. A disabled [`Style`] produces zero escape bytes, and no line is
//! wider than the width asked for.

mod blocks;
pub(in crate::render) mod edit_lines;
mod paint;
mod rows;
#[cfg(test)]
mod tests;
pub mod word_diff;

use jev_changes::domain::diff_parse::{FileDiff, Hunk};
use crate::render::style::Style;
use crate::render::width::{clamp_width, truncate_to_width};
use paint::{lang_of, number_width};
use rows::{split_rows, unified_rows};

/// Terminals at least this wide get the side-by-side layout when the layout is `Auto`.
pub const SPLIT_MIN_WIDTH: usize = 110;

/// Body lines of one file shown before the rest is folded, unless told otherwise.
pub const FOLD_AFTER: usize = 60;

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
        Self {
            width,
            layout: Layout::Auto,
            fold: Fold::After(FOLD_AFTER),
        }
    }

    fn split(&self) -> bool {
        match self.layout {
            Layout::Unified => false,
            Layout::Split => true,
            Layout::Auto => clamp_width(self.width) >= SPLIT_MIN_WIDTH,
        }
    }
}

fn counts(added: usize, removed: usize, style: Style) -> String {
    format!(
        "{} {}",
        style.green(&format!("+{added}")),
        style.red(&format!("−{removed}"))
    )
}

fn file_row(file: &FileDiff, style: Style, width: usize) -> String {
    truncate_to_width(
        &format!(
            "{} {}  {}",
            style.cyan("▸"),
            style.bold(&file.path),
            counts(file.added(), file.removed(), style)
        ),
        width,
    )
}

fn fold_row(hidden: usize, style: Style) -> String {
    let noun = if hidden == 1 { "line" } else { "lines" };
    style.dim(&format!(
        "  ⋯ {hidden} more {noun} folded (/diff --full to show)"
    ))
}

/// Rows for one hunk header: cyan, cut to the width.
fn hunk_row(hunk: &Hunk, style: Style, width: usize) -> String {
    truncate_to_width(&format!("  {}", style.cyan(&hunk.header)), width)
}

fn hunk_rows(file: &FileDiff, hunk: &Hunk, opts: &DiffOpts, style: Style) -> Vec<String> {
    let width = clamp_width(opts.width);
    let (lang, w) = (lang_of(&file.path), number_width(file));
    let mut rows = vec![hunk_row(hunk, style, width)];
    rows.extend(if opts.split() {
        split_rows(hunk, lang, w, style, width)
    } else {
        unified_rows(hunk, lang, w, style, width)
    });
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
    let all: Vec<Vec<String>> = file
        .hunks
        .iter()
        .map(|h| hunk_rows(file, h, opts, style))
        .collect();
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
    files
        .iter()
        .map(|f| render_file(f, opts, style).join("\n"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One hunk of one file, with its file row: what a per-hunk review shows.
pub fn render_hunk(file: &FileDiff, index: usize, opts: &DiffOpts, style: Style) -> Option<String> {
    let hunk = file.hunks.get(index)?;
    let mut rows = vec![truncate_to_width(
        &format!(
            "{} {}  {}",
            style.cyan("▸"),
            style.bold(&file.path),
            style.dim(&format!("hunk {} of {}", index + 1, file.hunks.len()))
        ),
        clamp_width(opts.width),
    )];
    rows.extend(hunk_rows(file, hunk, opts, style));
    Some(rows.join("\n"))
}
