use super::*;
use jev_backend::infrastructure::mcp_discovery::{McpRow, McpTransport};
use jev_backend::infrastructure::mcp_server::group;

mod input;
mod keys;
mod render;

fn row(name: &str, scope: McpScope, transport: McpTransport) -> McpRow {
    McpRow {
        name: name.into(),
        scope,
        transport,
        target: format!("https://{name}.example/mcp"),
    }
}

/// luna: http under claude and codex; pw: stdio under claude; posthog: http under claude.
fn servers() -> Vec<McpServer> {
    group(&[
        row("luna", McpScope::Claude, McpTransport::Http),
        row("pw", McpScope::Claude, McpTransport::Stdio),
        row("luna", McpScope::Codex, McpTransport::Http),
        row("posthog", McpScope::Claude, McpTransport::Http),
    ])
}

fn text(p: &McpPicker) -> String {
    p.render(70, Style::off()).join("\n")
}
