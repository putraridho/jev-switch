//! How each kind of thing in the interactive conversation looks, so a glance tells them
//! apart:
//!
//! - what **you** sent: a shaded band with `❯` (a rule and `❯` without colour);
//! - **tool calls**: dim, under a `⏺` marker, with what came back on a `⎿` row under it and
//!   an edit's lines as a small diff;
//! - the model's **narration** between tool calls: plain bright text, it is a message to
//!   you, set apart by a blank line and indented with the tool rows;
//! - the **answer**: plain bright text, set apart by a blank line.
//!
//! Colour is never the only cue: each role also has its own marker, indent or weight.

use std::path::Path;

use crate::tui::render::ansi::{clean_text, strip_ansi};
use crate::tui::render::changes::relativize_paths;
use crate::tui::render::route::{short_reason, RouteFacts};
use crate::tui::render::style::{Style, Theme};
use crate::tui::render::text::one_line;
use crate::tui::render::width::{clamp_width, display_width, truncate_to_width};
use crate::tui::render::wrap::wrap_to_width;

/// The most lines of an edit shown under it; the rest are counted.
pub const EDIT_PREVIEW_LINES: usize = 6;

/// The message you sent, as it stays in the scrollback: with colour, a full-width shaded
/// band, a blank band row above and below the text, `❯` on its first row and the time it was
/// sent (`stamp`, `6:27 PM`) flush right; the one block of background in the scrollback, so
/// your prompts are easy to find. Without colour, `❯ text` under a rule between turns.
/// `first` leaves out the separation above.
pub fn sent_block(text: &str, first: bool, stamp: Option<&str>, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let row_w = width.saturating_sub(1);
    let mut out = Vec::new();
    // With colour the band's own blank top row sets it apart; without, a rule does.
    if !first && !style.enabled {
        out.push("─".repeat(row_w));
    }
    let stamp = stamp.filter(|_| style.enabled).unwrap_or("");
    // Text wraps clear of the stamp: `  ❯ ` before it, `  6:27 PM  ` after.
    let right = if stamp.is_empty() { 2 } else { display_width(stamp) + 4 };
    let blank = style.band(&" ".repeat(row_w));
    if style.enabled {
        out.push(blank.clone());
    }
    let mut first_row = true;
    for line in text.split('\n') {
        for part in wrap_to_width(line, row_w.saturating_sub(4 + right).max(1)) {
            let lead = if first_row { "❯" } else { " " };
            if style.enabled {
                let tail = if first_row && !stamp.is_empty() { format!("{}  ", style.dim(stamp)) } else { String::new() };
                let fill = row_w.saturating_sub(4 + display_width(&part) + display_width(&strip_ansi(&tail)));
                out.push(style.band(&format!("  {} {}{}{tail}", style.accent(lead), style.bold(&part), " ".repeat(fill))));
            } else {
                out.push(format!("{lead} {part}"));
            }
            first_row = false;
        }
    }
    if style.enabled {
        out.push(blank);
    }
    out.push(String::new());
    out.join("\n")
}

/// Where a turn went: `  ⇄ claude · sonnet-5-5   feature · switched from codex (limit)`.
/// The provider in its colour, the router's reason in a few words. This is the router at
/// work, so it leads every turn; `⇄` is the router's mark wherever it shows.
pub fn route_row(f: &RouteFacts, style: Style, width: usize) -> String {
    let th = Theme::new(style);
    let who = if f.model.is_empty() { th.name(&f.provider) } else { format!("{} {}", th.name(&f.provider), style.dim(&format!("· {}", f.model))) };
    let why = short_reason(&f.reason);
    let why = if why.is_empty() { f.task.clone() } else { why };
    let mark = style.dim("⇄");
    let line = if why.is_empty() { format!("  {mark} {who}") } else { format!("  {mark} {who}   {}", style.dim(&why)) };
    truncate_to_width(&line, clamp_width(width))
}

/// A tool's name as a person reads it: an MCP tool `mcp__jev__delegate` is the worker
/// delegation, `Delegate`; another server's `mcp__srv__tool` is `srv·tool`.
pub fn display_tool(name: &str) -> String {
    let Some(rest) = name.strip_prefix("mcp__") else { return name.to_string() };
    let (server, tool) = rest.split_once("__").unwrap_or(("", rest));
    if server == "jev" {
        let mut c = tool.chars();
        return c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default().replace('_', " ");
    }
    if server.is_empty() { tool.to_string() } else { format!("{server}·{tool}") }
}

fn is_delegate(tool: &str) -> bool {
    tool == "delegate" || tool.ends_with("__delegate")
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

/// What a tool does, for counting a run of calls in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Edit,
    File,
    Search,
    Directory,
    Fetch,
    Command,
    Other,
}

fn tool_kind(tool: &str) -> ToolKind {
    match display_tool(tool).to_ascii_lowercase().as_str() {
        "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" | "replace" | "write_file" | "edit_file" => ToolKind::Edit,
        "read" | "read_file" | "read_many_files" | "view" => ToolKind::File,
        "grep" | "glob" | "search" | "search_file_content" | "find" | "websearch" | "web_search" | "google_web_search" => ToolKind::Search,
        "ls" | "list_directory" | "list" => ToolKind::Directory,
        "webfetch" | "web_fetch" | "fetch" => ToolKind::Fetch,
        "bash" | "shell" | "exec" | "run_shell_command" | "exec_command" => ToolKind::Command,
        _ => ToolKind::Other,
    }
}

/// A run of tool calls between two things the model said, counted for one summary row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolGroup {
    /// Files an edit changed or created, by name, in the order first seen; `true` = created.
    files: Vec<(String, bool)>,
    added: usize,
    removed: usize,
    edit_calls: usize,
    reads: usize,
    searches: usize,
    directories: usize,
    fetches: usize,
    commands: usize,
    others: usize,
    failed: usize,
}

impl ToolGroup {
    /// One call, by its tool's name (`Read`, `Bash`, `mcp__srv__tool`).
    pub fn call(&mut self, tool: &str) {
        match tool_kind(tool) {
            ToolKind::Edit => self.edit_calls += 1,
            ToolKind::File => self.reads += 1,
            ToolKind::Search => self.searches += 1,
            ToolKind::Directory => self.directories += 1,
            ToolKind::Fetch => self.fetches += 1,
            ToolKind::Command => self.commands += 1,
            ToolKind::Other => self.others += 1,
        }
    }

    /// What an edit did to a file: lines taken out and put in. No lines taken out is a new file.
    pub fn edit(&mut self, path: &str, removed: usize, added: usize) {
        let name = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        if !self.files.iter().any(|(f, _)| *f == name) {
            self.files.push((name, removed == 0));
        }
        self.added += added;
        self.removed += removed;
    }

    pub fn failed(&mut self) {
        self.failed += 1;
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The run as phrases: `Edited a.rs`, `explored 1 directory, 3 files`, `ran 2 commands`.
    fn phrases(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.files.is_empty() || self.edit_calls > 0 {
            let verb = if !self.files.is_empty() && self.files.iter().all(|(_, created)| *created) { "Created" } else { "Edited" };
            let what = match self.files.len() {
                0 => plural(self.edit_calls, "file", "files"),
                1 | 2 => self.files.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>().join(", "),
                n => plural(n, "file", "files"),
            };
            out.push(format!("{verb} {what}"));
        }
        let explored: Vec<String> = [(self.directories, "directory", "directories"), (self.reads, "file", "files"), (self.searches, "search", "searches"), (self.fetches, "fetch", "fetches")]
            .into_iter()
            .filter(|(n, _, _)| *n > 0)
            .map(|(n, one, many)| plural(n, one, many))
            .collect();
        if !explored.is_empty() {
            out.push(format!("explored {}", explored.join(", ")));
        }
        if self.commands > 0 {
            out.push(format!("ran {}", plural(self.commands, "command", "commands")));
        }
        if self.others > 0 {
            out.push(format!("called {}", plural(self.others, "tool", "tools")));
        }
        out
    }
}

/// A run of tool calls as one row, the way it reads in a sentence:
/// `  ◇ Edited parser.rs, explored 3 files, 1 search, ran 2 commands +12 −4`. The leading verb
/// in bold, the rest dim, lines added green and taken out red, failures counted in red.
pub fn tool_group_row(group: &ToolGroup, style: Style, width: usize) -> String {
    let mut phrases = group.phrases();
    if phrases.is_empty() {
        return String::new();
    }
    let first = crate::tui::render::text::capitalised(&phrases.remove(0));
    let (verb, rest) = first.split_once(' ').unwrap_or((&first, ""));
    let mut body = style.bold(verb);
    let tail: Vec<String> = std::iter::once(rest.to_string()).filter(|r| !r.is_empty()).chain(phrases).collect();
    if !tail.is_empty() {
        body.push_str(&style.dim(&format!(" {}", tail.join(", "))));
    }
    if group.added > 0 {
        body.push_str(&format!(" {}", style.green(&format!("+{}", group.added))));
    }
    if group.removed > 0 {
        body.push_str(&format!(" {}", style.red(&format!("−{}", group.removed))));
    }
    if group.failed > 0 {
        body.push_str(&format!(" {}", style.red(&format!("· {} failed", group.failed))));
    }
    truncate_to_width(&format!("  {} {body}", style.dim("◇")), clamp_width(width))
}

/// Whether a call went wrong: it failed, or it is a command whose output reports a failure.
pub fn tool_failed(tool: &str, ok: bool, text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    !ok || (tool_kind(tool) == ToolKind::Command && reports_failure(&lines))
}

/// A worker's result, from the delegate tool's first lines: `worker w1 (gemini argon): done`
/// and `files changed (2):` become `⇉ worker w1 · gemini argon · done · 2 files`.
fn worker_result(ok: bool, text: &str, style: Style) -> String {
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

/// The tool's name: the first word of its summary (`Bash cargo test` → `Bash`).
pub fn tool_name(summary: &str) -> &str {
    summary.split_whitespace().next().unwrap_or_default()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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
fn result_parts(tool: &str, ok: bool, text: &str, style: Style) -> Option<(String, String)> {
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
fn reports_failure(lines: &[&str]) -> bool {
    lines.iter().any(|l| {
        let l = l.trim_start();
        l.contains("FAILED") || l.starts_with("error") || l.starts_with("Error") || l.contains("panicked at") || l.starts_with("FAIL ")
    })
}

/// The lines an edit takes out (red, `-`) and puts in (green, `+`), at most
/// [`EDIT_PREVIEW_LINES`], the rest counted; a whole new file says how long it is.
pub fn edit_preview(removed: &[String], added: &[String], style: Style, width: usize) -> String {
    edit_preview_at(removed, added, None, style, width)
}

/// [`edit_preview`] with the line the change starts at: each row then carries its line number
/// in a gutter, and with colour the rows are tinted (a muted red and green band), so the
/// change reads as a diff at a glance. The `-`/`+` stay, so it reads without colour too.
pub fn edit_preview_at(removed: &[String], added: &[String], start: Option<usize>, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut rows = Vec::new();
    if removed.is_empty() {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("new file, {}", plural(added.len(), "line", "lines")))));
    } else {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("+{} −{}", added.len(), removed.len()))));
    }
    let gutter_w = start.map_or(0, |s| (s + removed.len().max(added.len())).to_string().len());
    let numbered = |n: usize| if gutter_w == 0 { String::new() } else { style.dim(&format!("{:>gutter_w$} ", n)) };
    let shown: Vec<(bool, String, usize)> = removed
        .iter()
        .enumerate()
        .map(|(i, l)| (false, format!("- {}", clean_text(l)), i))
        .chain(added.iter().enumerate().map(|(i, l)| (true, format!("+ {}", clean_text(l)), i)))
        .collect();
    let total = shown.len();
    for (is_added, text, i) in shown.into_iter().take(EDIT_PREVIEW_LINES) {
        let n = numbered(start.unwrap_or(0) + i);
        // The tint runs the width of the row, so the change is one band, not ragged text.
        let body = truncate_to_width(&text, width.saturating_sub(6 + if gutter_w == 0 { 0 } else { gutter_w + 1 }));
        let pad = if style.enabled { " ".repeat(width.saturating_sub(7 + if gutter_w == 0 { 0 } else { gutter_w + 1 }).saturating_sub(display_width(&body))) } else { String::new() };
        let colored = if is_added { style.green(&body) } else { style.red(&body) };
        let band = style.tint(is_added, &format!("{colored}{pad}"));
        rows.push(format!("      {n}{band}"));
    }
    if total > EDIT_PREVIEW_LINES {
        rows.push(style.dim(&format!("      … {} more lines · /diff after the turn", total - EDIT_PREVIEW_LINES)));
    }
    rows.join("\n")
}

/// What the model says between tool calls: it is a message to you, so full brightness (the
/// tool rows around it are the dim ones), indented with them.
pub fn narration(text: &str, _style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut rows = Vec::new();
    for line in clean_text(text).trim().lines() {
        for part in wrap_to_width(line, width.saturating_sub(4)) {
            rows.push(format!("  {part}"));
        }
    }
    rows.join("\n")
}

/// The most lines of the model's reasoning shown; the rest are counted.
pub const THINKING_LINES: usize = 3;

/// The trace reasoning leaves in the scrollback: `  ◆ Thought · its latest line`, dim, one row.
pub fn thought_row(latest: &str, style: Style, width: usize) -> String {
    let latest = one_line(latest);
    let tail = if latest.is_empty() { String::new() } else { style.dim(&style.italic(&format!(" · {latest}"))) };
    truncate_to_width(&format!("  {} {}{tail}", style.dim("◆"), style.dim(&style.bold("Thought"))), clamp_width(width))
}

/// The model's reasoning, when asked for in full: `  ◆ Thought`, then its first lines dim
/// italic under it, the rest counted.
pub fn thinking_block(text: &str, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();
    for line in clean_text(text).trim().lines().filter(|l| !l.trim().is_empty()) {
        lines.extend(wrap_to_width(line.trim(), width.saturating_sub(6)));
    }
    let total = lines.len();
    let mut rows = vec![format!("  {} {}", style.dim("◆"), style.dim(&style.bold("Thought")))];
    for line in lines.into_iter().take(THINKING_LINES) {
        rows.push(format!("    {}", style.dim(&style.italic(&line))));
    }
    if total > THINKING_LINES {
        let more = total - THINKING_LINES;
        rows.push(format!("    {}", style.dim(&format!("… {} of thinking", plural(more, "more line", "more lines")))));
    }
    rows.join("\n")
}

/// The answer on the interactive screen: plain bright text on the margin the rows around it
/// share, set apart by a blank line. Brightness against the dim rows is what marks it.
pub fn answer_panel(laid_out: &str, _style: Style) -> String {
    answer_block(laid_out, 2)
}

/// The answer: laid out text, set apart by a blank line above it and indented `margin`
/// columns so it lines up with the rows around it.
pub fn answer_block(laid_out: &str, margin: usize) -> String {
    let pad = " ".repeat(margin);
    let body: Vec<String> = laid_out.trim_end_matches('\n').split('\n').map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }).collect();
    format!("\n{}", body.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::strip_ansi;
    use crate::tui::render::test_facts::*;

    #[test]
    fn a_sent_message_is_a_shaded_band_or_a_plain_marker_without_colour() {
        assert_eq!(sent_block("fix it", true, Some("6:27 PM"), OFF, 40), "❯ fix it\n", "no stamp without colour");
        assert_eq!(sent_block("one\ntwo", false, None, OFF, 20), format!("{}\n❯ one\n  two\n", "─".repeat(19)));
        let band = sent_block("x", false, Some("6:27 PM"), ON, 40);
        let rows: Vec<&str> = band.lines().collect();
        assert!(rows[0].contains("\x1b[48;5;236") && strip_ansi(rows[0]).trim().is_empty(), "a blank band row above the text sets it apart");
        assert!(strip_ansi(rows[1]).starts_with("  ❯ x") && strip_ansi(rows[1]).ends_with("6:27 PM  "), "{:?}", strip_ansi(rows[1]));
        assert!(strip_ansi(rows[2]).trim().is_empty() && rows[2].contains("\x1b[48;5;236"), "and below it");
        for row in &rows[..3] {
            assert_eq!(crate::tui::render::display_width(&strip_ansi(row)), 39);
        }
        let long = sent_block(&"word ".repeat(20), true, Some("6:27 PM"), ON, 40);
        assert!(long.lines().all(|r| crate::tui::render::display_width(&strip_ansi(r)) <= 39), "wrapped clear of the stamp: {long}");
    }

    #[test]
    fn a_run_of_calls_reads_as_one_sentence() {
        let mut g = ToolGroup::default();
        assert!(g.is_empty() && tool_group_row(&g, OFF, 80).is_empty());
        for t in ["Read", "Read", "Grep", "LS", "Bash", "Bash", "WebFetch", "mcp__github__create_issue", "Edit"] {
            g.call(t);
        }
        g.edit("/p/src/parser.rs", 4, 12);
        assert_eq!(tool_group_row(&g, OFF, 120), "  ◇ Edited parser.rs, explored 1 directory, 2 files, 1 search, 1 fetch, ran 2 commands, called 1 tool +12 −4");
        let mut new = ToolGroup::default();
        new.call("Write");
        new.edit("notes.txt", 0, 3);
        new.call("Bash");
        new.failed();
        assert_eq!(tool_group_row(&new, OFF, 80), "  ◇ Created notes.txt, ran 1 command +3 · 1 failed");
        let on = tool_group_row(&new, ON, 80);
        assert!(on.contains("\x1b[1mCreated") && on.contains("\x1b[32m+3") && on.contains("\x1b[31m· 1 failed"), "{on:?}");
        let mut reads = ToolGroup::default();
        reads.call("Read");
        assert_eq!(tool_group_row(&reads, OFF, 80), "  ◇ Explored 1 file");
        let mut many = ToolGroup::default();
        for f in ["a.rs", "b.rs", "c.rs"] {
            many.call("Edit");
            many.edit(f, 1, 1);
        }
        assert_eq!(tool_group_row(&many, OFF, 80), "  ◇ Edited 3 files +3 −3");
        assert!(crate::tui::render::display_width(&tool_group_row(&g, OFF, 30)) <= 30);
        assert!(tool_failed("Bash", true, "test result: FAILED. 1 failed") && tool_failed("Read", false, "") && !tool_failed("Read", true, "FAILED"));
    }

    #[test]
    fn a_failed_call_under_the_run_says_what_and_how_it_failed() {
        let out = "running 4 tests\ntest empty ... FAILED\n\ntest result: FAILED. 3 passed; 1 failed";
        let rows = tool_failure_rows("Bash cargo test parser", true, out, Path::new("/p"), OFF, 80);
        assert_eq!(rows, vec!["    ✗ Bash cargo test parser", "      test result: FAILED. 3 passed; 1 failed  (+2 lines)", "      running 4 tests", "      test empty ... FAILED"]);
        let rows = tool_failure_rows("Read /p/a.rs", false, "no such file", Path::new("/p"), ON, 80);
        assert!(rows[0].contains("\x1b[31m✗") && strip_ansi(&rows[0]) == "    ✗ Read a.rs" && rows.len() == 2, "{rows:?}");
    }

    #[test]
    fn tool_rows_are_dim_with_the_name_plain_and_paths_relative() {
        let row = tool_row("Read /p/src/a.rs", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⏺ Read src/a.rs");
        let styled = tool_row("Read /p/src/a.rs", Path::new("/p"), ON, 80);
        assert!(styled.contains("Read") && styled.contains("\x1b[2msrc/a.rs"));
        assert!(crate::tui::render::display_width(&tool_row(&"x".repeat(300), Path::new("/p"), OFF, 40)) <= 40);
    }

    #[test]
    fn results_say_what_the_person_wants_to_know() {
        let r = |tool: &str, ok: bool, text: &str| tool_result_row(tool, ok, text, OFF, 80);
        assert_eq!(r("Read", true, "1→a\n2→b\n3→c").as_deref(), Some("    ⎿ ✓ read 3 lines"));
        assert_eq!(r("Grep", true, "").as_deref(), Some("    ⎿ ✓ no matches"));
        assert_eq!(r("Glob", true, "a\nb").as_deref(), Some("    ⎿ ✓ 2 paths"));
        assert_eq!(r("Bash", true, "running 3 tests\n\ntest result: ok. 3 passed").as_deref(), Some("    ⎿ ✓ test result: ok. 3 passed  (+1 lines)"));
        assert_eq!(r("shell", true, "").as_deref(), Some("    ⎿ ✓ done, no output"));
        assert_eq!(r("Bash", false, "error: no such file\nmore").as_deref(), Some("    ⎿ ✗ error: no such file"));
        assert_eq!(r("Edit", true, "The file was updated"), None, "the diff above says it");
        let failing = tool_result_row("Bash", true, "test empty ... FAILED\ntest result: FAILED. 3 passed; 1 failed", ON, 80).unwrap_or_default();
        assert!(failing.contains("\x1b[33mtest result: FAILED"), "a failing test run is yellow, not a quiet dim line: {failing:?}");
        assert!(tool_result_row("Bash", false, "boom", ON, 80).is_some_and(|s| s.contains("\x1b[31mboom")));
    }

    #[test]
    fn a_route_row_leads_the_turn_with_the_provider_and_why() {
        let f = RouteFacts { provider: "codex".into(), model: "gpt-6.1-sol".into(), reason: "task: feature (0.9) → codex gpt-6.1-sol; first turn".into(), ..RouteFacts::default() };
        assert_eq!(route_row(&f, OFF, 80), "  ⇄ codex · gpt-6.1-sol   feature · first turn");
        assert!(route_row(&f, ON, 80).contains("\x1b[32mcodex"), "codex in its colour");
    }

    #[test]
    fn delegation_has_its_own_mark_and_a_worker_result_says_who_did_what() {
        let row = tool_row("mcp__jev__delegate write the parser tests", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⇉ Delegate write the parser tests");
        assert_eq!(tool_row("mcp__github__create_issue x", Path::new("/p"), OFF, 80), "  ⏺ github·create_issue x");
        let done = "worker w1 (gemini gemini-4-argon): done\nfiles changed (2):\n  a.rs +3 −1\nworker report:\nok";
        assert_eq!(tool_result_row("mcp__jev__delegate", true, done, OFF, 80).as_deref(), Some("    ⎿ ✓ worker w1 · gemini gemini-4-argon · done · 2 files"));
        let failed = "worker w2 (codex gpt-6): FAILED\nerror: limit";
        let row = tool_result_row("mcp__jev__delegate", true, failed, ON, 80).unwrap_or_default();
        assert!(row.contains("\x1b[31mFAILED"), "{row:?}");
        assert_eq!(display_tool("mcp__jev__show_diff"), "Show diff");
    }

    const EVIL: &str = "a\x1b]52;c;QUFB\x07b\x1b]8;;http://x\x1b\\c\x1b[2Jd\u{9b}2Je\rf";

    fn assert_safe(out: &str) {
        let plain = strip_ansi(out);
        for bad in ['\x07', '\r', '\u{9b}', '\u{9d}', '\x08'] {
            assert!(!out.contains(bad), "{bad:?} in {out:?}");
        }
        assert!(!out.contains("]52") && !out.contains("]8;") && !out.contains("[2J"), "{out:?}");
        assert!(plain.contains("abcd"), "{plain:?}");
    }

    #[test]
    fn untrusted_text_in_output_edits_and_reasoning_is_neutralised() {
        let out = tool_output_fold("Bash", false, &format!("first\n{EVIL}"), ON, 80).unwrap_or_default();
        assert_safe(&out);
        assert!(out.contains("\x1b[2m"), "our colour stays: {out:?}");
        let edit = edit_preview(&[EVIL.to_string()], &[EVIL.to_string()], ON, 80);
        assert_safe(&edit);
        assert!(edit.contains("\x1b[31m") && edit.contains("\x1b[32m"));
        assert_safe(&thinking_block(EVIL, ON, 80));
        assert!(thinking_block(EVIL, ON, 80).contains("\x1b[3m"));
        assert_safe(&narration(EVIL, ON, 80));
    }

    #[test]
    fn failing_output_is_folded_under_the_result_row_and_quiet_output_is_not() {
        let long = (0..9).map(|i| format!("error line {i}")).collect::<Vec<_>>().join("\n");
        let out = tool_output_fold("Bash", false, &long, OFF, 80).unwrap_or_default();
        let rows: Vec<&str> = out.lines().collect();
        assert_eq!(rows.len(), FOLD_KEEP + 1, "{out}");
        assert_eq!(rows[0], "      error line 1", "line 0 is on the result row above");
        assert_eq!(rows[FOLD_KEEP], "      … +4 lines (output cut)");
        assert!(tool_output_fold("Bash", true, "a\nb\nc", OFF, 80).is_none());
        assert!(tool_output_fold("Bash", false, "one line", OFF, 80).is_none());
    }

    #[test]
    fn an_edit_with_a_known_start_numbers_its_lines_and_tints_them_with_colour() {
        let removed = vec!["let a = 1;".to_string()];
        let added = vec!["let a = 2;".to_string(), "let b = 3;".to_string()];
        assert_eq!(edit_preview_at(&removed, &added, Some(9), OFF, 80), "    ⎿ +2 −1\n       9 - let a = 1;\n       9 + let a = 2;\n      10 + let b = 3;");
        let on = edit_preview_at(&removed, &added, Some(9), ON, 40);
        assert!(on.contains("\x1b[48;5;52") && on.contains("\x1b[48;5;22"), "{on:?}");
        for row in on.lines().skip(1) {
            assert!(crate::tui::render::display_width(&strip_ansi(row)) <= 40, "{row:?}");
        }
        assert_eq!(edit_preview_at(&removed, &added, None, OFF, 80), edit_preview(&removed, &added, OFF, 80));
    }

    #[test]
    fn an_edit_shows_a_small_diff_and_counts_the_rest() {
        let removed = vec!["let a = 1;".to_string()];
        let added = vec!["let a = 2;".to_string(), "let b = 3;".to_string()];
        assert_eq!(edit_preview(&removed, &added, OFF, 80), "    ⎿ +2 −1\n      - let a = 1;\n      + let a = 2;\n      + let b = 3;");
        let many: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let new_file = edit_preview(&[], &many, OFF, 80);
        assert!(new_file.starts_with("    ⎿ new file, 10 lines\n      + line 0"), "{new_file}");
        assert!(new_file.ends_with("… 4 more lines · /diff after the turn"), "{new_file}");
    }

    #[test]
    fn thinking_shows_its_first_lines_behind_a_mark_and_counts_the_rest() {
        assert_eq!(thinking_block("one\n\ntwo", OFF, 80), "  ◆ Thought\n    one\n    two");
        let long = thinking_block("a\nb\nc\nd\ne", OFF, 80);
        assert_eq!(long, "  ◆ Thought\n    a\n    b\n    c\n    … 2 more lines of thinking");
        assert_eq!(thought_row("check the\ncallers", OFF, 80), "  ◆ Thought · check the callers");
        assert_eq!(thought_row("", OFF, 80), "  ◆ Thought");
        assert!(thinking_block("a\nb\nc\nd", OFF, 80).ends_with("… 1 more line of thinking"));
    }

    #[test]
    fn narration_is_plain_bright_text_and_the_answer_is_set_apart() {
        assert_eq!(narration("Let me look.\n", OFF, 80), "  Let me look.");
        assert!(!narration("x", ON, 80).contains("\x1b["), "not dimmed: it is for reading");
        assert_eq!(answer_block("The answer.\n", 0), "\nThe answer.");
        assert_eq!(answer_block("One.\n\nTwo.\n", 2), "\n  One.\n\n  Two.");
        no_esc(&strip_ansi(&narration("a", ON, 80)));
    }
}
