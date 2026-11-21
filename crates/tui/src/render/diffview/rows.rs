use super::blocks::{blocks_of, Block, Cell};
use super::paint::{marker, num, paint_text};
use jev_changes::domain::diff_parse::{Hunk, LineKind};
use crate::render::highlight::{Lang, LineState};
use crate::render::style::Style;
use crate::render::width::{pad_to_width, truncate_to_width};

/// One diff row in the unified layout: `old new ± text`.
fn unified_row(
    old: Option<u32>,
    new: Option<u32>,
    kind: LineKind,
    text: String,
    w: usize,
    style: Style,
    width: usize,
) -> String {
    let line = format!(
        "  {} {} {} {}",
        num(old, w, style),
        num(new, w, style),
        marker(kind, style),
        text
    );
    truncate_to_width(line.trim_end(), width)
}

/// The rows of one hunk, unified.
pub(super) fn unified_rows(
    hunk: &Hunk,
    lang: Option<Lang>,
    w: usize,
    style: Style,
    width: usize,
) -> Vec<String> {
    let mut state = LineState::default();
    let mut rows = Vec::new();
    for block in blocks_of(hunk) {
        match block {
            Block::Same { old, new, text } => {
                let t = paint_text(&text, None, LineKind::Context, lang, &mut state, style);
                rows.push(unified_row(
                    Some(old),
                    Some(new),
                    LineKind::Context,
                    t,
                    w,
                    style,
                    width,
                ));
            }
            Block::Change { removed, added } => {
                for c in &removed {
                    let t = paint_text(
                        &c.text,
                        c.pieces.as_deref(),
                        c.kind,
                        lang,
                        &mut state,
                        style,
                    );
                    rows.push(unified_row(
                        Some(c.number),
                        None,
                        c.kind,
                        t,
                        w,
                        style,
                        width,
                    ));
                }
                for c in &added {
                    let t = paint_text(
                        &c.text,
                        c.pieces.as_deref(),
                        c.kind,
                        lang,
                        &mut state,
                        style,
                    );
                    rows.push(unified_row(
                        None,
                        Some(c.number),
                        c.kind,
                        t,
                        w,
                        style,
                        width,
                    ));
                }
            }
        }
    }
    rows
}

/// One half of a side-by-side row, exactly `half` columns wide.
fn half_cell(
    number: Option<u32>,
    kind: LineKind,
    text: &str,
    w: usize,
    half: usize,
    style: Style,
) -> String {
    let body = if text.is_empty() && number.is_none() {
        String::new()
    } else {
        format!("{} {} {}", num(number, w, style), marker(kind, style), text)
    };
    pad_to_width(&truncate_to_width(&body, half), half)
}

/// The rows of one hunk, side by side: old on the left, new on the right.
pub(super) fn split_rows(
    hunk: &Hunk,
    lang: Option<Lang>,
    w: usize,
    style: Style,
    width: usize,
) -> Vec<String> {
    let half = (width.saturating_sub(2 + 3) / 2).max(1);
    let sep = style.dim(" │ ");
    let mut state = LineState::default();
    let mut rows = Vec::new();
    let mut row = |l: String, r: String| rows.push(format!("  {l}{sep}{r}").trim_end().to_string());
    for block in blocks_of(hunk) {
        match block {
            Block::Same { old, new, text } => {
                let t = paint_text(&text, None, LineKind::Context, lang, &mut state, style);
                row(
                    half_cell(Some(old), LineKind::Context, &t, w, half, style),
                    half_cell(Some(new), LineKind::Context, &t, w, half, style),
                );
            }
            Block::Change { removed, added } => {
                for i in 0..removed.len().max(added.len()) {
                    let side = |c: Option<&Cell>, state: &mut LineState| match c {
                        Some(c) => half_cell(
                            Some(c.number),
                            c.kind,
                            &paint_text(&c.text, c.pieces.as_deref(), c.kind, lang, state, style),
                            w,
                            half,
                            style,
                        ),
                        None => half_cell(None, LineKind::Context, "", w, half, style),
                    };
                    let (l, r) = (
                        side(removed.get(i), &mut state),
                        side(added.get(i), &mut state),
                    );
                    row(l, r);
                }
            }
        }
    }
    rows
}
