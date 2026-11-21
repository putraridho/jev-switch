use super::word_diff::{word_diff, Piece};
use crate::changes::domain::diff_parse::LineKind;
use crate::tui::render::style::Style;

/// The `- old` / `+ new` lines of an edit approval card, coloured: removed red, added
/// green, and when a run of removed lines is followed by as many added ones the words that
/// differ are reversed. One entry per input line, `None` for a line that is not a change
/// line (the path row, `edit 2:`) or when colour is off. A painted entry has the text of
/// its line exactly; only escapes are added.
pub(in crate::tui::render) fn paint_edit_lines(
    lines: &[String],
    style: Style,
) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; lines.len()];
    if !style.enabled {
        return out;
    }
    fn side(l: &str) -> Option<(LineKind, &str)> {
        l.strip_prefix("- ")
            .map(|t| (LineKind::Removed, t))
            .or_else(|| l.strip_prefix("+ ").map(|t| (LineKind::Added, t)))
    }
    let paint = |kind: LineKind, marker: &str, pieces: Option<&[Piece]>, text: &str| {
        let color = |s: &str| {
            if kind == LineKind::Removed {
                style.red(s)
            } else {
                style.green(s)
            }
        };
        let body = match pieces {
            Some(p) => p
                .iter()
                .map(|p| {
                    if p.changed {
                        style.reverse(&color(&p.text))
                    } else {
                        color(&p.text)
                    }
                })
                .collect(),
            None => color(text),
        };
        format!("{}{body}", color(marker))
    };
    let mut i = 0;
    while i < lines.len() {
        let run = |from: usize, kind: LineKind| {
            lines[from..]
                .iter()
                .take_while(|l| side(l).is_some_and(|(k, _)| k == kind))
                .count()
        };
        let removed = run(i, LineKind::Removed);
        let added = run(i + removed, LineKind::Added);
        if removed + added == 0 {
            i += 1;
            continue;
        }
        let paired = removed == added;
        for k in 0..removed + added {
            let Some((kind, text)) = side(&lines[i + k]) else {
                continue;
            };
            let marker = if kind == LineKind::Removed {
                "- "
            } else {
                "+ "
            };
            let other = if paired {
                if k < removed {
                    side(&lines[i + k + removed])
                } else {
                    side(&lines[i + k - removed])
                }
            } else {
                None
            };
            let pieces = other.and_then(|(_, o)| {
                if kind == LineKind::Removed {
                    word_diff(text, o).map(|p| p.0)
                } else {
                    word_diff(o, text).map(|p| p.1)
                }
            });
            out[i + k] = Some(paint(kind, marker, pieces.as_deref(), text));
        }
        i += removed + added;
    }
    out
}
