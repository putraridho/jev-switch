//! A resumed session on the screen: the turns it saved, laid out the way they looked when they
//! ran (what you sent, which model answered, the tools it used, the answer), so `/resume` and
//! `-c` bring the conversation back and not only a count of it.

use jev_session::domain::turn_record::TurnRecord;

use crate::render::{answer_panel, answer_text, route_row, sent_block, truncate_to_width, RouteFacts, Style};

/// Turns shown when a session is longer; the earlier ones stay in the session, only off the screen.
pub const SHOWN_TURNS: usize = 30;
/// Tool rows shown under one turn before "and N more".
const TOOL_ROWS: usize = 6;

/// The saved turns as scrollback text, oldest first; empty for a session with no turns.
pub fn history_block(turns: &[TurnRecord], style: Style, width: usize) -> String {
    let skipped = turns.len().saturating_sub(SHOWN_TURNS);
    let mut out: Vec<String> = Vec::new();
    if skipped > 0 {
        out.push(style.dim(&format!("  … {skipped} earlier turns not shown")));
    }
    for (i, t) in turns.iter().enumerate().skip(skipped) {
        out.push(sent_block(t.prompt.trim(), i == 0, None, style, width));
        let facts = RouteFacts { provider: t.provider.to_string(), model: t.model.clone(), ..RouteFacts::default() };
        out.push(route_row(&facts, style, width));
        for tool in t.tools.iter().take(TOOL_ROWS) {
            out.push(truncate_to_width(&format!("    {} {}", style.dim("⎿"), style.dim(tool)), width));
        }
        if t.tools.len() > TOOL_ROWS {
            out.push(style.dim(&format!("    ⎿ and {} more tool calls", t.tools.len() - TOOL_ROWS)));
        }
        let reply = t.reply.trim();
        if !reply.is_empty() {
            out.push(answer_panel(&answer_text(reply, style, width.saturating_sub(4)), style));
        }
        if !t.ok {
            out.push(style.dim("  (did not finish)"));
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::strip_ansi;
    use jev_shared::ProviderId;

    fn turn(prompt: &str, reply: &str) -> TurnRecord {
        TurnRecord { prompt: prompt.into(), provider: ProviderId::Claude, model: "claude-sonnet-5-5".into(), reply: reply.into(), tools: vec!["Read src/a.rs".into()], ok: true }
    }

    #[test]
    fn a_resumed_session_shows_what_was_asked_and_answered() {
        let text = strip_ansi(&history_block(&[turn("fix the bug", "Done, it was the cache."), turn("and the test", "Added.")], Style::off(), 60));
        for want in ["❯ fix the bug", "claude · claude-sonnet-5-5", "⎿ Read src/a.rs", "Done, it was the cache.", "❯ and the test", "Added."] {
            assert!(text.contains(want), "missing {want:?} in {text}");
        }
    }

    #[test]
    fn a_long_session_shows_its_latest_turns_and_says_so() {
        let turns: Vec<TurnRecord> = (0..SHOWN_TURNS + 5).map(|i| turn(&format!("q{i}"), "a")).collect();
        let text = history_block(&turns, Style::off(), 60);
        assert!(text.contains("5 earlier turns not shown") && text.contains("q34") && !text.contains("❯ q0\n"));
        assert_eq!(history_block(&[], Style::off(), 60), "");
    }
}
