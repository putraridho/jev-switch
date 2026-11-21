//! What a tool returned: the `⎿` row, a failing call's rows and the folded output.

use std::path::Path;

use super::group::{tool_kind, ToolKind};
use super::tool::{display_tool, is_delegate, plural, tool_name};
use crate::tui::render::ansi::clean_text;
use crate::tui::render::changes::relativize_paths;
use crate::tui::render::style::{Style, Theme};
use crate::tui::render::text::one_line;
use crate::tui::render::width::{clamp_width, truncate_to_width};

/// Whether a call went wrong: it failed, or it is a command whose output reports a failure.
pub fn tool_failed(tool: &str, ok: bool, text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    !ok || (tool_kind(tool) == ToolKind::Command && reports_failure(&lines))
}

/// A worker's result, from the delegate tool's first lines: `worker w1 (gemini argon): done`
/// and `files changed (2):` become `⇉ worker w1 · gemini argon · done · 2 files`.
pub(super) fn worker_result(ok: bool, text: &str, style: Style) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let parsed = first.strip_prefix("worker ").and_then(|r| {
        let (id, rest) = r.split_once(" (")?;
        let (who, outcome) = rest.split_once("): ")?;
        Some((id, who, outcome))
    });
    let Some((id, who, outcome)) = parsed else {
        return if ok { style.dim(&one_line(first)) } else { style.red(&one_line(first)) };
    };
    let files = text.lines().find_map(|l| {
        let l = l.trim();
        if l == "files changed: none" {
            return Some("no files".to_string());
        }
        let n = l.strip_prefix("files changed (")?.split(')').next()?.parse::<usize>().ok()?;
        Some(plural(n, "file", "files"))
    });
    let provider = who.split_whitespace().next().unwrap_or(who);
    let th = Theme::new(style);
    let done = outcome.trim() == "done" && ok;
    let state = if done { style.green("done") } else { style.red(outcome.trim()) };
    let mut s = format!("worker {id} {} {} {} {state}", style.dim("·"), th.provider(provider, who), style.dim("·"));
    if let Some(f) = files {
        s.push_str(&style.dim(&format!(" · {f}")));
    }
    s
}

/// What a tool returned, in a few words, as the person wants it: how much was read, how many
/// matches, the line of a command's output that says how it went; an error's first line in
/// red. `None` when the line above says it all (an edit shows its own diff).
pub fn tool_result_row(tool: &str, ok: bool, text: &str, style: Style, width: usize) -> Option<String> {
    let (status, said) = result_parts(tool, ok, text, style)?;
    Some(truncate_to_width(&format!("    {} {status} {said}", style.dim("⎿")), clamp_width(width)))
}

/// A call that went wrong, under the run's row: `    ✗ Bash cargo test`, the line that says
/// what happened under it, then the output folded to its first lines.
pub fn tool_failure_rows(summary: &str, ok: bool, text: &str, project: &Path, style: Style, width: usize) -> Vec<String> {
    let width = clamp_width(width);
    let tool = tool_name(summary);
    let body = relativize_paths(&one_line(summary), project);
    let (verb, detail) = body.split_once(' ').unwrap_or((&body, ""));
    let mark = if ok { style.yellow("✗") } else { style.red("✗") };
    let mut rows = vec![truncate_to_width(format!("    {mark} {} {}", style.bold(&display_tool(verb)), style.dim(detail)).trim_end(), width)];
    if let Some((_, said)) = result_parts(tool, ok, text, style) {
        rows.push(truncate_to_width(&format!("      {said}"), width));
    }
    rows.extend(tool_output_fold(tool, ok, text, style, width).into_iter().flat_map(|f| f.lines().map(str::to_string).collect::<Vec<_>>()));
    rows
}

/// What a result row says: its status mark and the words. `None` when an edit's own diff says it.
pub(super) fn result_parts(tool: &str, ok: bool, text: &str, style: Style) -> Option<(String, String)> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tool_lc = tool.to_ascii_lowercase();
    let said = if is_delegate(&tool_lc) {
        worker_result(ok, text, style)
    } else if !ok {
        let first = lines.first().map_or("failed", |l| l.trim());
        style.red(&one_line(first))
    } else {
        match tool_lc.as_str() {
            "edit" | "multiedit" | "write" | "notebookedit" | "todowrite" => return None,
            "read" => style.dim(&format!("read {}", plural(lines.len(), "line", "lines"))),
            "grep" => style.dim(&if lines.is_empty() { "no matches".to_string() } else { plural(lines.len(), "match", "matches") }),
            "glob" | "ls" => style.dim(&if lines.is_empty() { "nothing found".to_string() } else { plural(lines.len(), "path", "paths") }),
            _ if lines.is_empty() => style.dim("done, no output"),
            "bash" | "shell" => {
                // A command says how it went at the end (`test result: ok. 12 passed`).
                let last = lines.last().map_or("", |l| l.trim());
                let more = if lines.len() > 1 { format!("  (+{} lines)", lines.len() - 1) } else { String::new() };
                let text = format!("{}{more}", one_line(last));
                // Output that reports a failure is not a quiet success, whatever the exit code.
                if reports_failure(&lines) { style.yellow(&text) } else { style.dim(&text) }
            }
            _ => {
                let first = lines.first().map_or("", |l| l.trim());
                let more = if lines.len() > 1 { format!("  (+{} lines)", lines.len() - 1) } else { String::new() };
                style.dim(&format!("{}{more}", one_line(first)))
            }
        }
    };
    let status = if !ok {
        style.red("✗")
    } else if matches!(tool_lc.as_str(), "bash" | "shell") && reports_failure(&lines) {
        style.yellow("✗")
    } else {
        style.green("✓")
    };
    Some((status, said))
}

/// How many lines of a failing command's output show before the fold.
pub const FOLD_KEEP: usize = 4;

/// The output of a failed tool, or a command whose output reports a failure, folded to its
/// first [`FOLD_KEEP`] lines under the `⎿` row with a note counting the rest. `None` for
/// output that went well or is one line (the `⎿` row already says it).
pub fn tool_output_fold(tool: &str, ok: bool, text: &str, style: Style, width: usize) -> Option<String> {
    let lines: Vec<String> = clean_text(text).lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim_end().to_string()).collect();
    let tool_lc = tool.to_ascii_lowercase();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let shell = matches!(tool_lc.as_str(), "bash" | "shell");
    if lines.len() < 2 || is_delegate(&tool_lc) || !(!ok || (shell && reports_failure(&refs))) {
        return None;
    }
    let width = clamp_width(width);
    // The `⎿` row above already quotes one line (a command's last, an error's first): don't
    // print it a second time under itself.
    let rest: Vec<String> = if ok { lines[..lines.len() - 1].to_vec() } else { lines[1..].to_vec() };
    let folded = crate::tui::render::collapse::fold(&rest, FOLD_KEEP, "output cut");
    let rows: Vec<String> = folded
        .iter()
        .map(|l| truncate_to_width(&format!("      {}", style.dim(l)), width))
        .collect();
    Some(rows.join("\n"))
}

/// Whether a command's output says something failed: a test run with failures, a compiler
/// error, a panic.
pub(super) fn reports_failure(lines: &[&str]) -> bool {
    lines.iter().any(|l| {
        let l = l.trim_start();
        l.contains("FAILED") || l.starts_with("error") || l.starts_with("Error") || l.contains("panicked at") || l.starts_with("FAIL ")
    })
}
