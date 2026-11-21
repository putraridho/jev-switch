/// Two lines count as "the same line edited" for word highlighting when at least this
/// share (percent) of the longer one is unchanged; otherwise marking words is noise.
const MIN_SIMILARITY_PERCENT: usize = 40;

/// Most words per line the word diff will compare (it is quadratic).
const MAX_WORD_TOKENS: usize = 200;

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
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
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
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    t
}

fn push_piece(pieces: &mut Vec<Piece>, text: &str, changed: bool) {
    match pieces.last_mut() {
        Some(last) if last.changed == changed => last.text.push_str(text),
        _ => pieces.push(Piece {
            text: text.to_string(),
            changed,
        }),
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
