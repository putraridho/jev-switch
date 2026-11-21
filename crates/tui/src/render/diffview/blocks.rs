use super::word_diff::{word_diff, Piece};
use jev_changes::domain::diff_parse::{Hunk, LineKind};

/// One line of one side of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Cell {
    pub(super) number: u32,
    pub(super) kind: LineKind,
    pub(super) text: String,
    /// Set when this line pairs with an edited line on the other side.
    pub(super) pieces: Option<Vec<Piece>>,
}

/// A stretch of a hunk: unchanged lines, or a change (lines removed, lines added).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Block {
    Same {
        old: u32,
        new: u32,
        text: String,
    },
    Change {
        removed: Vec<Cell>,
        added: Vec<Cell>,
    },
}

fn expand_tabs(s: &str) -> String {
    s.replace('\t', "    ")
}

pub(super) fn blocks_of(hunk: &Hunk) -> Vec<Block> {
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
        blocks.push(Block::Change {
            removed: std::mem::take(removed),
            added: std::mem::take(added),
        });
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
                removed.push(Cell {
                    number: old,
                    kind: LineKind::Removed,
                    text,
                    pieces: None,
                });
                old += 1;
            }
            LineKind::Added => {
                added.push(Cell {
                    number: new,
                    kind: LineKind::Added,
                    text,
                    pieces: None,
                });
                new += 1;
            }
            LineKind::NoNewline => {}
        }
    }
    flush(&mut blocks, &mut removed, &mut added);
    blocks
}
