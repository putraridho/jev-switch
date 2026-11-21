use super::{clean, dot, Action, McpPicker};
use crate::backend::infrastructure::mcp_server::McpHealth;
use crate::tui::render::{truncate_to_width, Style};

impl McpPicker {
    pub(super) fn render_detail(
        &self,
        server: usize,
        action: usize,
        width: usize,
        style: Style,
    ) -> Vec<String> {
        let s = &self.servers[server];
        let fit = |text: &str, indent: usize| {
            truncate_to_width(&clean(text), width.saturating_sub(indent))
        };
        let mut out = vec![style.bold(&fit(&s.name, 0)), String::new()];
        out.push(format!(
            "  {} {}",
            style.dim("Target:   "),
            fit(&s.target, 13)
        ));
        out.push(format!("  {} {}", style.dim("Transport:"), s.transport));
        out.push(String::new());
        out.push(format!("  {}", style.dim("Providers")));
        let scope_col = s
            .presences
            .iter()
            .map(|p| p.scope.to_string().len())
            .max()
            .unwrap_or(0);
        for p in &s.presences {
            let scope = format!("{:<scope_col$}", p.scope.to_string());
            let label = p.health.label();
            let mut line = format!("    {} {scope}  {label}", dot(&p.health, style));
            if let McpHealth::Failed(why) = &p.health {
                let used = 4 + 2 + scope_col + 2 + label.len() + 3;
                line.push_str(&style.dim(&format!(" — {}", fit(why, used))));
            }
            out.push(line);
        }
        out.push(String::new());
        out.push(format!("  {}", style.dim("Actions")));
        let actions = Self::actions(s);
        if actions.len() == 1 {
            out.push(format!(
                "    {}",
                style.dim(&fit(
                    "No login needed (stdio, or no claude/codex config).",
                    4
                ))
            ));
        }
        for (i, a) in actions.iter().enumerate() {
            let text = match a {
                Action::Login(scope) => format!("Log in with {scope}"),
                Action::Back => "Back".to_string(),
            };
            out.push(if i == action {
                format!("  {} {}", style.cyan("❯"), style.bold(&text))
            } else {
                format!("    {text}")
            });
        }
        out.push(String::new());
        out.push(style.dim(&truncate_to_width(
            "  ↑↓ choose · Enter run · ←/Esc back",
            width,
        )));
        out
    }
}
