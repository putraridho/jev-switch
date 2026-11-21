use super::*;

/// `JEV_SHOW_THINKING=1` seeds whether the model's whole reasoning is printed; `/thinking`
/// changes it for the session and keeps the choice.
pub(super) fn show_thinking_from_env() -> bool {
    std::env::var("JEV_SHOW_THINKING").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// The line an edit starts at, found by reading the file (never writing it): where the text
/// being replaced is now, or where the new text is when the edit has already landed. `None`
/// when neither is there (a new file, a path outside the project, a file too large to scan).
pub(super) fn edit_start_line(project: &std::path::Path, path: &str, removed: &[String], added: &[String]) -> Option<usize> {
    let file = std::path::Path::new(path);
    let full = if file.is_absolute() { file.to_path_buf() } else { project.join(file) };
    if std::fs::metadata(&full).ok()?.len() > 2_000_000 {
        return None;
    }
    let text = std::fs::read_to_string(&full).ok()?;
    [removed, added].into_iter().filter(|b| !b.is_empty()).find_map(|block| {
        let needle = block.join("\n");
        text.find(&needle).map(|at| text[..at].matches('\n').count() + 1)
    })
}

/// `JEV_SHOW_NARRATION=1` prints what the model says between tool calls into the scrollback.
pub(super) fn show_narration() -> bool {
    std::env::var("JEV_SHOW_NARRATION").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// The answer gutter and attribution header are opt-in: `JEV_CODE_GUTTER=1`. Off by default
/// so existing users' answers (and the PTY scenarios) keep their layout.
pub(super) fn attribution_gutter_enabled() -> bool {
    gutter_setting(std::env::var("JEV_CODE_GUTTER").ok().as_deref())
}

pub(super) fn gutter_setting(value: Option<&str>) -> bool {
    value == Some("1")
}

/// A plain engine note ("[jev] unavailable ...") as a warning line: the bracketed tag
/// becomes a prefix (`jev: unavailable ...`).
pub fn note_line(line: &str, style: Style) -> String {
    let line = line.trim();
    let text = match line.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((tag, rest)) => format!("{tag}: {rest}"),
        None => line.to_string(),
    };
    format!("  {} {text}", style.yellow("⚠"))
}

/// Whether a queued line is a message for the model. A command (`/model`, `!ls`, a custom
/// `/name`) is run by the session after the turn and is never written into the turn.
pub(super) fn steerable_text(text: &str) -> bool {
    !text.trim_start().starts_with(['/', '!']) && matches!(crate::command::parse(text), crate::command::Command::Prompt(_))
}

/// `↳ message (taken into the running turn)`: a queued message the model has now.
pub(super) fn steered_line(text: &str, style: Style) -> String {
    let shown = text.lines().next().unwrap_or_default();
    let more = if text.lines().count() > 1 { " …" } else { "" };
    format!("  {} {}{more} {}", style.cyan("↳"), shown, style.dim("(taken into the running turn)"))
}

/// `JEV_TOOL_ROWS=all` prints every tool call as it comes, with its result and an edit's diff,
/// instead of one row per run.
pub(super) fn group_tool_runs() -> bool {
    std::env::var("JEV_TOOL_ROWS").map_or(true, |v| !v.trim().eq_ignore_ascii_case("all"))
}

/// Calls that always print on their own: what is handed to a worker (orchestration is the
/// thing to watch).
pub(super) fn stands_alone(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t == "delegate" || t.ends_with("__delegate")
}

/// The note that a call was allowed without asking (`✓ auto-allowed …`).
pub(super) fn is_allow_note(text: &str) -> bool {
    let flat = uifmt::strip_ansi(text);
    flat.trim_start().starts_with("\u{2713} auto-allowed ") || flat.trim_start().starts_with("\u{2713} allowed (remembered) ")
}

/// A hint as one yellow line that fits the terminal: a long command or message is cut with `…`
/// instead of filling the screen (the tool row above it already shows the whole call).
pub(super) fn hint_line(text: &str, style: Style, width: usize) -> String {
    let flat = uifmt::strip_ansi(text).split_whitespace().collect::<Vec<_>>().join(" ");
    // A call that went through unasked is a quiet note under its tool row, which already
    // shows the whole call: repeating the command made every call twice as noisy.
    for (prefix, short) in [("\u{2713} auto-allowed ", "auto-allowed"), ("\u{2713} allowed (remembered) ", "allowed (remembered)")] {
        if flat.starts_with(prefix) {
            return format!("    {}", style.dim(&format!("\u{2713} {short}")));
        }
    }
    format!("  {}", style.yellow(&uifmt::truncate_to_width(&flat, width.saturating_sub(2))))
}

/// The provider's name in its colour, for lines built outside this module.
pub fn provider_name(provider: &str, style: Style) -> String {
    Theme::new(style).name(provider)
}
