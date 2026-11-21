//! Owns the prompt text: the routing label, the plain and the coloured prompt, the mode line.
//! Must not read the line editor.

use crate::routing::domain::decide::Pin;
use crate::backend::ProviderId;
use crate::tui::render as uifmt;
use crate::tui::render::Style;

/// What the prompt says about routing: the pin (`codex`, `claude:claude-opus-5-5`), or
/// `claude only` when routing is off; `None` when the router is free to choose.
pub fn prompt_label(pin: &Pin, routing_on: bool) -> Option<String> {
    match pin {
        Pin::Auto => (!routing_on).then(|| "claude only".to_string()),
        other => Some(other.to_string()),
    }
}

/// `auto (accepts edits and safe commands, asks about the rest)`.
pub fn mode_line(mode: crate::approval::domain::mode::Mode) -> String {
    format!("{mode} ({})", mode.about())
}

/// The prompt as plain text, which is what the line editor measures: `› ` or `[codex] › `.
pub fn plain_prompt(label: Option<&str>) -> String {
    match label {
        Some(l) => format!("[{l}] › "),
        None => "› ".to_string(),
    }
}

/// The same prompt coloured: the label in the provider's colour, the arrow bold.
pub fn styled_prompt(label: Option<&str>, style: Style) -> String {
    let arrow = style.cyan(&style.bold("›"));
    match label {
        Some(l) => {
            let provider = l.split(':').next().unwrap_or(l);
            let tag = if provider.parse::<ProviderId>().is_ok() {
                uifmt::Theme::new(style).provider(provider, &format!("[{l}]"))
            } else {
                style.dim(&format!("[{l}]"))
            };
            format!("{tag} {arrow} ")
        }
        None => format!("{arrow} "),
    }
}
