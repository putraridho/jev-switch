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

use super::ansi::clean_text;
use super::changes::relativize_paths;
use super::route::{short_reason, RouteFacts};
use super::style::{Style, Theme};
use super::text::one_line;
use super::width::{clamp_width, display_width, truncate_to_width};
use super::wrap::wrap_to_width;

/// The most lines of an edit shown under it; the rest are counted.
pub const EDIT_PREVIEW_LINES: usize = 6;

/// The message you sent, as it stays in the scrollback: with colour, a full-width shaded
/// band with `❯` on its first row, the one block of background in the scrollback so your
/// prompts are easy to find; without colour, `❯ text` under a rule between turns. `first`
/// leaves out the separation above.
pub fn sent_block(text: &str, first: bool, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut out = Vec::new();
    if !first {
        out.push(if style.enabled { String::new() } else { "─".repeat(width.saturating_sub(1)) });
    }
    for (i, line) in text.split('\n').enumerate() {
        for (j, part) in wrap_to_width(line, width.saturating_sub(5)).iter().enumerate() {
            let lead = if i == 0 && j == 0 { "❯" } else { " " };
            if style.enabled {
                let fill = width.saturating_sub(1).saturating_sub(display_width(part) + 3);
                out.push(style.band(&format!(" {} {part}{}", style.accent(lead), " ".repeat(fill))));
            } else {
                out.push(format!("{lead} {part}"));
            }
        }
    }
    out.push(String::new());
    out.join("\n")
}

/// Where a turn went: `  ◆ claude · sonnet-5-5   feature · switched from codex (limit)`.
/// The provider in its colour, the router's reason in a few words. This is the router at
/// work, so it leads every turn.
pub fn route_row(f: &RouteFacts, style: Style, width: usize) -> String {
    let th = Theme::new(style);
    let who = if f.model.is_empty() { th.name(&f.provider) } else { format!("{} {} {}", th.name(&f.provider), style.dim("·"), f.model) };
    let why = short_reason(&f.reason);
    let why = if why.is_empty() { f.task.clone() } else { why };
    let line = if why.is_empty() { format!("  {} {who}", th.provider(&f.provider, "◆")) } else { format!("  {} {who}   {}", th.provider(&f.provider, "◆"), style.dim(&why)) };
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
    let mark = if is_delegate(verb) { style.cyan("⇉") } else { style.dim("⏺") };
    let verb = display_tool(verb);
    let line = if detail.is_empty() { format!("  {mark} {verb}") } else { format!("  {mark} {verb} {}", style.dim(detail)) };
    truncate_to_width(&line, clamp_width(width))
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
    Some(truncate_to_width(&format!("    {} {said}", style.dim("⎿")), clamp_width(width)))
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
    let folded = crate::workstation::collapse::fold(&lines, FOLD_KEEP, "output cut");
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
    let width = clamp_width(width);
    let mut rows = Vec::new();
    if removed.is_empty() {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("new file, {}", plural(added.len(), "line", "lines")))));
    } else {
        rows.push(format!("    {} {}", style.dim("⎿"), style.dim(&format!("+{} −{}", added.len(), removed.len()))));
    }
    let shown: Vec<String> = removed
        .iter()
        .map(|l| style.red(&format!("- {}", clean_text(l))))
        .chain(added.iter().map(|l| style.green(&format!("+ {}", clean_text(l)))))
        .collect();
    let total = shown.len();
    for line in shown.into_iter().take(EDIT_PREVIEW_LINES) {
        rows.push(truncate_to_width(&format!("      {line}"), width));
    }
    if total > EDIT_PREVIEW_LINES {
        rows.push(style.dim(&format!("      … {} more lines · /diff after the turn", total - EDIT_PREVIEW_LINES)));
    }
    rows.join("\n")
}

/// What the model says between tool calls: it is a message to you, so full brightness (the
/// tool rows around it are the dim ones), indented with them.
pub fn narration(text: &str, style: Style, width: usize) -> String {
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

/// The model's reasoning: `  ✻ ` and its first lines, dim italic, the rest counted.
pub fn thinking_block(text: &str, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();
    for line in clean_text(text).trim().lines().filter(|l| !l.trim().is_empty()) {
        lines.extend(wrap_to_width(line.trim(), width.saturating_sub(6)));
    }
    let total = lines.len();
    let mut rows = Vec::new();
    for (i, line) in lines.into_iter().take(THINKING_LINES).enumerate() {
        let lead = if i == 0 { style.dim("✻") } else { " ".to_string() };
        rows.push(format!("  {lead} {}", style.dim(&style.italic(&line))));
    }
    if total > THINKING_LINES {
        let more = total - THINKING_LINES;
        rows.push(format!("    {}", style.dim(&format!("… {} of thinking", plural(more, "more line", "more lines")))));
    }
    rows.join("\n")
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
    use crate::uifmt::strip_ansi;
    use crate::uifmt::testutil::*;

    #[test]
    fn a_sent_message_is_a_shaded_band_or_a_plain_marker_without_colour() {
        assert_eq!(sent_block("fix it", true, OFF, 40), "❯ fix it\n");
        assert_eq!(sent_block("one\ntwo", false, OFF, 20), format!("{}\n❯ one\n  two\n", "─".repeat(19)));
        let band = sent_block("x", false, ON, 40);
        let rows: Vec<&str> = band.lines().collect();
        assert_eq!(rows[0], "", "a blank line separates turns");
        assert!(rows[1].contains("\x1b[48;5;236") && strip_ansi(rows[1]).contains("❯ x"));
        assert_eq!(crate::uifmt::display_width(&strip_ansi(rows[1])), 39);
    }

    #[test]
    fn tool_rows_are_dim_with_the_name_plain_and_paths_relative() {
        let row = tool_row("Read /p/src/a.rs", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⏺ Read src/a.rs");
        let styled = tool_row("Read /p/src/a.rs", Path::new("/p"), ON, 80);
        assert!(styled.contains("Read") && styled.contains("\x1b[2msrc/a.rs"));
        assert!(crate::uifmt::display_width(&tool_row(&"x".repeat(300), Path::new("/p"), OFF, 40)) <= 40);
    }

    #[test]
    fn results_say_what_the_person_wants_to_know() {
        let r = |tool: &str, ok: bool, text: &str| tool_result_row(tool, ok, text, OFF, 80);
        assert_eq!(r("Read", true, "1→a\n2→b\n3→c").as_deref(), Some("    ⎿ read 3 lines"));
        assert_eq!(r("Grep", true, "").as_deref(), Some("    ⎿ no matches"));
        assert_eq!(r("Glob", true, "a\nb").as_deref(), Some("    ⎿ 2 paths"));
        assert_eq!(r("Bash", true, "running 3 tests\n\ntest result: ok. 3 passed").as_deref(), Some("    ⎿ test result: ok. 3 passed  (+1 lines)"));
        assert_eq!(r("shell", true, "").as_deref(), Some("    ⎿ done, no output"));
        assert_eq!(r("Bash", false, "error: no such file\nmore").as_deref(), Some("    ⎿ error: no such file"));
        assert_eq!(r("Edit", true, "The file was updated"), None, "the diff above says it");
        let failing = tool_result_row("Bash", true, "test empty ... FAILED\ntest result: FAILED. 3 passed; 1 failed", ON, 80).unwrap_or_default();
        assert!(failing.contains("\x1b[33mtest result: FAILED"), "a failing test run is yellow, not a quiet dim line: {failing:?}");
        assert!(tool_result_row("Bash", false, "boom", ON, 80).is_some_and(|s| s.contains("\x1b[31mboom")));
    }

    #[test]
    fn a_route_row_leads_the_turn_with_the_provider_and_why() {
        let f = RouteFacts { provider: "codex".into(), model: "gpt-6.1-sol".into(), reason: "task: feature (0.9) → codex gpt-6.1-sol; first turn".into(), ..RouteFacts::default() };
        assert_eq!(route_row(&f, OFF, 80), "  ◆ codex · gpt-6.1-sol   feature · first turn");
        assert!(route_row(&f, ON, 80).contains("\x1b[32mcodex"), "codex in its colour");
    }

    #[test]
    fn delegation_has_its_own_mark_and_a_worker_result_says_who_did_what() {
        let row = tool_row("mcp__jev__delegate write the parser tests", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⇉ Delegate write the parser tests");
        assert_eq!(tool_row("mcp__github__create_issue x", Path::new("/p"), OFF, 80), "  ⏺ github·create_issue x");
        let done = "worker w1 (gemini gemini-4-argon): done\nfiles changed (2):\n  a.rs +3 −1\nworker report:\nok";
        assert_eq!(tool_result_row("mcp__jev__delegate", true, done, OFF, 80).as_deref(), Some("    ⎿ worker w1 · gemini gemini-4-argon · done · 2 files"));
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
        let out = tool_output_fold("Bash", false, &format!("{EVIL}\nsecond"), ON, 80).unwrap_or_default();
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
        assert_eq!(rows[0], "      error line 0");
        assert_eq!(rows[FOLD_KEEP], "      … +5 lines (output cut)");
        assert!(tool_output_fold("Bash", true, "a\nb\nc", OFF, 80).is_none());
        assert!(tool_output_fold("Bash", false, "one line", OFF, 80).is_none());
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
        assert_eq!(thinking_block("one\n\ntwo", OFF, 80), "  ✻ one\n    two");
        let long = thinking_block("a\nb\nc\nd\ne", OFF, 80);
        assert_eq!(long, "  ✻ a\n    b\n    c\n    … 2 more lines of thinking");
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
