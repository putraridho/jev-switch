//! The side panel next to the transcript: tasks, router state and changed files as sections,
//! and how they sit beside (or, on a narrow terminal, under) the main rows.

use crate::uifmt::{display_width, truncate_to_width, Style};

/// Terminals narrower than this stack the panel under the transcript instead of beside it.
pub const SPLIT_MIN_WIDTH: usize = 100;
/// The panel's columns when beside the transcript.
pub const PANEL_WIDTH: usize = 32;

/// One titled block of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<String>,
}

/// The panel's rows: each non-empty section a rounded box, its title in the top border
/// (`╭─ Router ───────╮`) and its rows inside, every row exactly `width` columns. Boxes sit
/// directly on one another, so the panel reads as a stack of labelled panes.
pub fn panel_rows(sections: &[Section], width: usize, style: Style) -> Vec<String> {
    let inner = width.saturating_sub(4).max(1);
    let mut out: Vec<String> = Vec::new();
    for s in sections.iter().filter(|s| !s.rows.is_empty()) {
        let title = truncate_to_width(&s.title, inner.saturating_sub(2));
        let fill = width.saturating_sub(display_width(&title) + 5);
        out.push(format!("{}{}{}", style.dim("╭─ "), style.bold(&title), style.dim(&format!(" {}╮", "─".repeat(fill)))));
        for r in &s.rows {
            let shown = truncate_to_width(r, inner);
            let pad = " ".repeat(inner - display_width(&shown));
            out.push(format!("{} {shown}{pad} {}", style.dim("│"), style.dim("│")));
        }
        out.push(style.dim(&format!("╰{}╯", "─".repeat(width.saturating_sub(2)))));
    }
    out
}

/// `main` rows with `side` rows beside them when `width` allows, else `side` under `main`
/// behind a rule. Every row is at most `width` columns.
pub fn compose(main: &[String], side: &[String], width: usize, style: Style) -> Vec<String> {
    if side.is_empty() {
        return main.iter().map(|r| truncate_to_width(r, width)).collect();
    }
    if width < SPLIT_MIN_WIDTH {
        let mut out: Vec<String> = main.iter().map(|r| truncate_to_width(r, width)).collect();
        out.push(style.dim(&"─".repeat(width)));
        out.extend(side.iter().map(|r| truncate_to_width(r, width)));
        return out;
    }
    let main_w = width - PANEL_WIDTH - 3;
    let bar = style.dim(" │ ");
    (0..main.len().max(side.len()))
        .map(|i| {
            let left = truncate_to_width(main.get(i).map_or("", String::as_str), main_w);
            let pad = " ".repeat(main_w - display_width(&left));
            let right = truncate_to_width(side.get(i).map_or("", String::as_str), PANEL_WIDTH);
            format!("{left}{pad}{bar}{right}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::strip_ansi;

    const OFF: Style = Style { enabled: false, terminal: false };

    fn sections() -> Vec<Section> {
        vec![
            Section { title: "Tasks".into(), rows: vec!["☐ write tests".into()] },
            Section { title: "Empty".into(), rows: vec![] },
            Section { title: "Changed".into(), rows: vec!["M src/lib.rs".into()] },
        ]
    }

    #[test]
    fn sections_stack_with_a_blank_between_and_empty_ones_vanish() {
        let rows = panel_rows(&sections(), 30, OFF);
        assert_eq!(rows[0], "╭─ Tasks ────────────────────╮");
        assert_eq!(rows[1], "│ ☐ write tests              │");
        assert_eq!(rows[2], format!("╰{}╯", "─".repeat(28)));
        assert!(rows[3].starts_with("╭─ Changed") && rows.len() == 6, "empty sections leave no box: {rows:?}");
        assert!(panel_rows(&sections(), 30, OFF).iter().all(|r| display_width(r) == 30));
        assert!(panel_rows(&sections(), 8, OFF).iter().all(|r| display_width(r) == 8));
    }

    #[test]
    fn wide_terminals_put_the_panel_beside_and_narrow_ones_under() {
        let main: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let side = panel_rows(&sections(), PANEL_WIDTH, OFF);
        let wide = compose(&main, &side, 120, OFF);
        assert_eq!(wide.len(), 6, "as tall as the panel when that is taller");
        assert!(wide[0].starts_with("line 0") && wide[0].contains("│ ╭─ Tasks"), "{:?}", wide[0]);
        assert!(wide.iter().all(|r| display_width(r) <= 120));
        let narrow = compose(&main, &side, 60, OFF);
        assert_eq!(narrow.len(), 5 + 1 + side.len());
        assert!(narrow[5].starts_with('─') && narrow[6].starts_with("╭─ Tasks"));
        assert!(narrow.iter().all(|r| display_width(r) <= 60));
    }

    #[test]
    fn colour_only_adds_escapes_and_no_panel_leaves_the_main_rows_alone() {
        let main = vec!["a".to_string()];
        assert_eq!(compose(&main, &[], 120, OFF), main);
        let on = Style { enabled: true, terminal: true };
        let styled = compose(&main, &panel_rows(&sections(), 30, on), 120, on);
        assert_eq!(strip_ansi(&styled[0]), compose(&main, &panel_rows(&sections(), 30, OFF), 120, OFF)[0]);
    }
}
