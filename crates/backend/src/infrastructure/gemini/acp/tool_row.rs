//! How a tool call reads on its row in the transcript.

use serde_json::Value;

/// How a call reads on its tool row: a shell command as `Bash <command>`, whichever field its
/// raw input names it by (`command`, Antigravity's `CommandLine`), so it reads as any other
/// provider's; else the tool's name and its most telling input. `None` while the call has
/// said nothing beyond its name. Display only: approval never reads a command from here.
pub(super) fn tool_row_summary(call: &Value) -> Option<String> {
    let raw = &call["rawInput"];
    let short = |s: &str| s.chars().take(100).collect::<String>();
    let name = call["name"].as_str().or(call["title"].as_str()).unwrap_or("tool");
    if let Some(command) = raw["command"].as_str().or(raw["CommandLine"].as_str()).filter(|c| !c.trim().is_empty()) {
        return Some(format!("Bash {}", short(command)));
    }
    let telling = ["TargetFile", "AbsolutePath", "SearchPath", "DirectoryPath", "Query", "Url", "file_path", "path", "pattern", "url", "query"]
        .iter()
        .find_map(|k| raw[*k].as_str())
        .filter(|t| !t.trim().is_empty());
    if let Some(t) = telling {
        return Some(format!("{name} {}", short(t)));
    }
    if raw.as_object().is_some_and(|o| !o.is_empty()) {
        return Some(name.to_string());
    }
    call["title"].as_str().filter(|t| !t.trim().is_empty() && Some(*t) != call["name"].as_str()).map(|t| format!("{name} {}", short(t)))
}
