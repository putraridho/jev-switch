//! The `/` menu above the input box.

use super::layout::spread;
use super::theme::Theme;
use crate::tui::input::menu::MENU_ROWS;
use crate::tui::render::{display_width, truncate_to_width};

/// The `/` menu, above the box: a heading row with the selected command's arguments and the
/// position (`/model [target]            1/59`), then the rows, the selected one raised.
pub(super) fn menu_rows(m: &crate::tui::input::menu::Menu, w: usize, th: &Theme) -> Vec<String> {
    let first = m.first_shown();
    // The label column fits the rows on screen, not the longest label anywhere.
    let label_w = m.items.iter().skip(first).take(MENU_ROWS).map(|i| display_width(&i.label)).max().unwrap_or(0).min(w / 2).max(8);
    let usage = m.items.get(m.selected).filter(|i| !i.usage.is_empty()).map(|i| format!("{} {}", th.accent(&i.label), th.muted(&i.usage))).unwrap_or_default();
    let position = if m.items.len() > MENU_ROWS { th.muted(&format!("{}/{}", m.selected + 1, m.items.len())) } else { String::new() };
    let mut rows = vec![spread(&format!("  {usage}"), &position, w)];
    for (i, item) in m.items.iter().enumerate().skip(first).take(MENU_ROWS) {
        let label = truncate_to_width(&item.label, label_w);
        let pad = " ".repeat(label_w.saturating_sub(display_width(&label)) + 2);
        let detail = truncate_to_width(&item.detail, w.saturating_sub(label_w + 8));
        if i == m.selected {
            let row = format!(" {} {}{pad}{detail}", th.accent("❯"), th.selected(&label));
            let fill = w.saturating_sub(display_width(&row));
            rows.push(th.raised(&format!("{row}{}", " ".repeat(fill))));
        } else {
            rows.push(truncate_to_width(&format!("   {label}{pad}{}", th.muted(&detail)), w));
        }
    }
    rows
}
