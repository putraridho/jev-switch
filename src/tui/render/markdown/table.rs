//! GitHub pipe tables drawn as aligned columns.

use crate::tui::render::style::Style;
use crate::tui::render::width::display_width;

/// How a table column is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

/// The cells of a `| a | b |` row, trimmed. A `\|` is part of a cell.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with("\\|") { t } else { t.strip_suffix('|').unwrap_or(t) };
    let (mut cells, mut cur, mut escaped) = (Vec::new(), String::new(), false);
    for c in t.chars() {
        if c == '|' && !escaped {
            cells.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    cells.push(cur.trim().to_string());
    cells
}

/// The alignment a separator cell (`---`, `:--`, `--:`, `:-:`) asks for.
fn parse_align(cell: &str) -> Option<Align> {
    let dashes = cell.trim_matches(':');
    if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
        return None;
    }
    Some(match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// `cell` padded with spaces to `width` columns by `align`, painted by `paint`.
fn pad_cell(cell: &str, width: usize, align: Align, paint: impl Fn(&str) -> String) -> String {
    let gap = width.saturating_sub(display_width(cell));
    let (left, right) = match align {
        Align::Left => (0, gap),
        Align::Right => (gap, 0),
        Align::Center => (gap / 2, gap - gap / 2),
    };
    format!("{}{}{}", " ".repeat(left), paint(cell), " ".repeat(right))
}

/// A table row's cells drawn in columns, with no trailing spaces.
fn draw_row(cells: &[String], widths: &[usize], aligns: &[Align], paint: impl Fn(&str) -> String) -> String {
    let empty = String::new();
    let drawn: Vec<String> =
        (0..widths.len()).map(|i| pad_cell(cells.get(i).unwrap_or(&empty), widths[i], aligns[i], &paint)).collect();
    drawn.join("  ").trim_end().to_string()
}

/// A GitHub pipe table (header, separator, rows) drawn as aligned columns with a dim
/// rule under the header. Anything that is not a table, or is wider than `width`, is
/// returned as the lines were written.
pub(super) fn render_table(lines: &[String], style: Style, width: usize) -> Vec<String> {
    let as_written = || lines.iter().map(|l| l.trim_end().to_string()).collect();
    let rows: Vec<Vec<String>> = lines.iter().map(|l| split_row(l)).collect();
    let Some(aligns) = rows.get(1).and_then(|r| r.iter().map(|c| parse_align(c)).collect::<Option<Vec<Align>>>()) else {
        return as_written();
    };
    let columns = aligns.len();
    let body: Vec<&Vec<String>> = rows.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, r)| r).collect();
    if rows[0].len() != columns || body.iter().any(|r| r.len() > columns) {
        return as_written();
    }
    let widths: Vec<usize> =
        (0..columns).map(|i| body.iter().filter_map(|r| r.get(i)).map(|c| display_width(c)).max().unwrap_or(0).max(1)).collect();
    if widths.iter().sum::<usize>() + 2 * (columns - 1) > width {
        return as_written();
    }
    let rule = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("  ");
    let mut out = vec![draw_row(&rows[0], &widths, &aligns, |c| style.bold(c)), style.dim(&rule)];
    out.extend(body.iter().skip(1).map(|r| draw_row(r, &widths, &aligns, |c| c.to_string())));
    out
}
