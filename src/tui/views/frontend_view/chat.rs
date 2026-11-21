//! The conversation pane of the full-screen layout.

use crate::tui::app::screen_state::App;
use crate::tui::render::{answer_panel, answer_text, truncate_to_width, Style};

/// The conversation rows for the chat pane, ending with the answer as it is being written.
/// The preview is shown only while following the end; it never enters the transcript, and
/// the finished answer replaces it when the turn commits.
pub(super) fn chat_rows(app: &App, width: usize, height: usize, style: Style) -> Vec<String> {
    let preview = app.live_answer.as_deref().filter(|t| !t.trim().is_empty()).filter(|_| app.transcript.scrolled() == 0);
    let Some(text) = preview else {
        return app.transcript.window(height).iter().map(|l| truncate_to_width(l, width)).collect();
    };
    let laid = answer_panel(&answer_text(text, style, width.saturating_sub(4)), style);
    let mut rows: Vec<String> = laid.split('\n').map(|l| truncate_to_width(l, width)).collect();
    // A long answer keeps its newest rows, like the conversation below the fold.
    if rows.len() > height {
        rows.drain(..rows.len() - height);
    }
    let above = height - rows.len();
    let mut all: Vec<String> = app.transcript.window(above).iter().map(|l| truncate_to_width(l, width)).collect();
    all.extend(rows);
    all
}
