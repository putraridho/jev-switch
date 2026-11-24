//! What the model says: narration, reasoning and the answer.

use super::tool::plural;
use crate::render::ansi::clean_text;
use crate::render::style::Style;
use crate::render::text::one_line;
use crate::render::width::clamp_width;
use crate::render::wrap::wrap_to_width;

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

/// The trace reasoning leaves in the scrollback: `  ◆ Thought · its latest line`, dim. The
/// line is wrapped under its first word, never cut.
pub fn thought_row(latest: &str, style: Style, width: usize) -> String {
    const LEAD: usize = 14; // "  ◆ Thought · "
    let latest = one_line(latest);
    let head = format!("  {} {}", style.dim("◆"), style.dim(&style.bold("Thought")));
    if latest.is_empty() {
        return head;
    }
    let parts = wrap_to_width(&latest, clamp_width(width).saturating_sub(LEAD).max(10));
    let mut rows = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            rows.push(format!("{head}{}", style.dim(&style.italic(&format!(" · {part}")))));
        } else {
            rows.push(format!("{}{}", " ".repeat(LEAD), style.dim(&style.italic(part))));
        }
    }
    rows.join("\n")
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
