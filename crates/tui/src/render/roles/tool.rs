//! One tool call as a row, and the names a tool goes by.

use std::path::Path;

use crate::render::changes::relativize_paths;
use crate::render::style::Style;
use crate::render::text::one_line;
use crate::render::width::{clamp_width, truncate_to_width};

/// A tool's name as a person reads it: an MCP tool `mcp__jev__delegate` is the worker
/// delegation, `Delegate`; another server's `mcp__srv__tool` is `srv·tool`.
pub fn display_tool(name: &str) -> String {
    let unadorned = name.strip_prefix("mcp__").unwrap_or(name);
    let (server, tool) = unadorned.split_once("__").unwrap_or(("", unadorned));
    if server == "jev" || tool == "delegate" {
        let mut c = tool.chars();
        return c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default().replace('_', " ");
    }
    match name.to_ascii_lowercase().as_str() {
        "agent" | "task" | "spawn_agent" | "invoke_subagent" | "subagent" => "Delegate".to_string(),
        _ => if server.is_empty() { tool.to_string() } else { format!("{server}·{tool}") }
    }
}

pub(super) fn is_delegate(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t == "delegate"
        || t.ends_with("__delegate")
        || matches!(t.as_str(), "agent" | "task" | "spawn_agent" | "invoke_subagent" | "subagent")
}

/// One tool call: `  ⏺ Bash cargo test`, the marker and the details dim, the tool's name
/// plain, paths inside the project relative. A piece handed to a worker is `  ⇉ Delegate`
/// with its brief: orchestration has its own mark.
pub fn tool_row(summary: &str, project: &Path, style: Style, width: usize) -> String {
    let body = relativize_paths(&one_line(summary), project);
    let (verb, detail) = body.split_once(' ').unwrap_or((&body, ""));
    let mark = if is_delegate(verb) { style.cyan("⇉") } else { style.accent("⏺") };
    let verb = style.bold(&display_tool(verb));
    let line = if detail.is_empty() { format!("  {mark} {verb}") } else { format!("  {mark} {verb} {}", style.dim(detail)) };
    truncate_to_width(&line, clamp_width(width))
}

/// The tool's name: the first word of its summary (`Bash cargo test` → `Bash`).
pub fn tool_name(summary: &str) -> &str {
    summary.split_whitespace().next().unwrap_or_default()
}

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
