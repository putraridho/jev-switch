use super::word_diff::Piece;
use jev_changes::domain::diff_parse::{FileDiff, LineKind};
use crate::render::highlight::{highlight_line, Lang, LineState};
use crate::render::style::Style;

pub(super) fn lang_of(path: &str) -> Option<Lang> {
    path.rsplit_once('.')
        .and_then(|(_, ext)| Lang::from_info(ext))
}

/// The text of a cell: edited lines in their side's colour with the changed words
/// reversed; everything else syntax-coloured when the language is known.
pub(super) fn paint_text(
    cell_text: &str,
    pieces: Option<&[Piece]>,
    kind: LineKind,
    lang: Option<Lang>,
    state: &mut LineState,
    style: Style,
) -> String {
    if !style.enabled {
        return cell_text.to_string();
    }
    if let Some(pieces) = pieces {
        let side = |s: &str| {
            if kind == LineKind::Removed {
                style.red(s)
            } else {
                style.green(s)
            }
        };
        return pieces
            .iter()
            .map(|p| {
                if p.changed {
                    style.reverse(&side(&p.text))
                } else {
                    side(&p.text)
                }
            })
            .collect();
    }
    match lang {
        Some(l) => highlight_line(l, cell_text, state, style),
        None => cell_text.to_string(),
    }
}

pub(super) fn marker(kind: LineKind, style: Style) -> String {
    match kind {
        LineKind::Added => style.bold(&style.green("+")),
        LineKind::Removed => style.bold(&style.red("-")),
        _ => " ".to_string(),
    }
}

pub(super) fn num(n: Option<u32>, w: usize, style: Style) -> String {
    style.dim(&format!(
        "{:>w$}",
        n.map_or(String::new(), |n| n.to_string())
    ))
}

fn digits(n: u32) -> usize {
    n.max(1).to_string().len()
}

/// Width of the line-number column for a file: the widest number any hunk reaches.
pub(super) fn number_width(file: &FileDiff) -> usize {
    let top = file
        .hunks
        .iter()
        .map(|h| (h.old_start + h.lines.len() as u32).max(h.new_start + h.lines.len() as u32))
        .max()
        .unwrap_or(1);
    digits(top)
}
