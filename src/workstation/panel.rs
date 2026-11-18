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

/// The panel's rows: each section's title in bold over its rows, a blank row between sections,
/// every row cut to `width` columns. A section with no rows is left out.
pub fn panel_rows(sections: &[Section], width: usize, style: Style) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in sections.iter().filter(|s| !s.rows.is_empty()) {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(truncate_to_width(&style.bold(&s.title), width));
        out.extend(s.rows.iter().map(|r| truncate_to_width(&format!("  {r}"), width)));
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
        assert_eq!(rows, vec!["Tasks", "  ☐ write tests", "", "Changed", "  M src/lib.rs"]);
        assert!(panel_rows(&sections(), 8, OFF).iter().all(|r| display_width(r) <= 8));
    }

    #[test]
    fn wide_terminals_put_the_panel_beside_and_narrow_ones_under() {
        let main: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let side = panel_rows(&sections(), PANEL_WIDTH, OFF);
        let wide = compose(&main, &side, 120, OFF);
        assert_eq!(wide.len(), 5);
        assert!(wide[0].starts_with("line 0") && wide[0].ends_with("│ Tasks"), "{:?}", wide[0]);
        assert!(wide.iter().all(|r| display_width(r) <= 120));
        let narrow = compose(&main, &side, 60, OFF);
        assert_eq!(narrow.len(), 5 + 1 + side.len());
        assert!(narrow[5].starts_with('─') && narrow[6] == "Tasks");
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
