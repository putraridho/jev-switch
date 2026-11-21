use super::{clean, dot, McpPicker};
use crate::backend::infrastructure::mcp_server::{McpHealth, McpServer};
use crate::tui::render::{display_width, truncate_to_width, Style};

impl McpPicker {
    pub(super) fn render_list(&self, width: usize, style: Style) -> Vec<String> {
        let shown = self.visible();
        let mut out = vec![style.bold("MCP servers"), String::new()];
        let search = if self.filter.is_empty() {
            style.dim("type to search")
        } else {
            truncate_to_width(&clean(&self.filter), width.saturating_sub(12))
        };
        out.push(format!("  {} {search}", style.dim("Search:")));
        out.push(String::new());
        if shown.is_empty() {
            out.push(format!("  {}", style.dim("No server matches.")));
        }
        // Names get at most half the line after the marker and dot, so the providers show too.
        let name_cap = (width.saturating_sub(6) / 2).clamp(4, 28);
        let name_col = shown
            .iter()
            .map(|&i| display_width(&clean(&self.servers[i].name)))
            .max()
            .unwrap_or(0)
            .clamp(4, name_cap);
        for (pos, &i) in shown.iter().enumerate().skip(self.offset).take(self.page) {
            out.push(self.list_row(
                &self.servers[i],
                pos == self.selected,
                name_col,
                width,
                style,
            ));
        }
        if shown.len() > self.page {
            let end = (self.offset + self.page).min(shown.len());
            out.push(style.dim(&format!("  {}-{} of {}", self.offset + 1, end, shown.len())));
        }
        out.push(String::new());
        if self.checking() {
            out.push(style.dim("  checking servers…"));
        }
        out.push(style.dim(&truncate_to_width(
            "  ↑↓ move · Enter/→ open · Esc close · type to search",
            width,
        )));
        out
    }

    /// `❯ ● name   http · claude · codex  needs login`, cut to the width.
    fn list_row(
        &self,
        s: &McpServer,
        selected: bool,
        name_col: usize,
        width: usize,
        style: Style,
    ) -> String {
        let marker = if selected {
            style.cyan("❯")
        } else {
            " ".to_string()
        };
        let name = truncate_to_width(&clean(&s.name), name_col);
        let pad = " ".repeat(name_col.saturating_sub(display_width(&name)) + 2);
        let name = if selected { style.bold(&name) } else { name };
        let scopes: Vec<String> = s.presences.iter().map(|p| p.scope.to_string()).collect();
        let note = match s.health() {
            McpHealth::NeedsLogin => "needs login",
            McpHealth::Failed(_) => "failed",
            _ => "",
        };
        // marker, space, dot, space, name, pad
        let used = 4 + name_col + 2;
        let room = width.saturating_sub(used);
        let meta_text = format!("{} · {}", s.transport, scopes.join(" · "));
        let (meta, note) = if note.is_empty() {
            (truncate_to_width(&meta_text, room), String::new())
        } else {
            let meta = truncate_to_width(&meta_text, room.saturating_sub(note.len() + 2));
            let left = room.saturating_sub(display_width(&meta) + 2);
            (meta, truncate_to_width(note, left))
        };
        let note = match s.health() {
            McpHealth::NeedsLogin if !note.is_empty() => format!("  {}", style.yellow(&note)),
            McpHealth::Failed(_) if !note.is_empty() => format!("  {}", style.red(&note)),
            _ => String::new(),
        };
        format!(
            "{marker} {} {name}{pad}{}{note}",
            dot(s.health(), style),
            style.dim(&meta)
        )
    }
}
