//! The `/context` "what fills it" rows, printed from a [`Breakdown`].

use crate::session::domain::context_intelligence::{AutoCompact, Breakdown};
use crate::shared::domain::wording::human_count;
use crate::tui::render::Style;

/// The "what fills it" part of `/context`: one row per source that has something in it.
pub fn breakdown_text(b: &Breakdown, auto: AutoCompact, style: Style) -> String {
    let total = b.total().max(1);
    let share = |n: u64| format!("{:>8} tokens  {:>3}%", human_count(n), n * 100 / total);
    let mut lines = vec![String::new(), style.bold("What fills it (estimated)")];
    let mut add = |label: &str, n: u64| {
        if n > 0 {
            lines.push(format!("  {}{}", style.dim(&format!("{label:<12}")), share(n)));
        }
    };
    add("your messages", b.prompts);
    add("answers", b.answers);
    add("waiting", b.waiting);
    add("memory", b.memory);
    if b.total() == 0 {
        lines.push(format!("  {}", style.dim("nothing yet")));
    }
    if let Some((n, tokens)) = b.heaviest_turn {
        lines.push(format!("  {}", style.dim(&format!("heaviest turn: {n} (about {})", human_count(tokens)))));
    }
    lines.push(format!(
        "  {}",
        style.dim(&match auto {
            AutoCompact::Off => "auto-compaction is off (JEV_AUTOCOMPACT=<percent> turns it on)".to_string(),
            AutoCompact::At(p) => format!("compacts itself at {p}% (JEV_AUTOCOMPACT=off or a percent)"),
        })
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::domain::context_intelligence::breakdown;
    use crate::session::domain::session_log::TurnRecord;
    use crate::session::domain::turn_status::TurnStatus;

    fn turn(prompt: &str, answer: &str) -> TurnRecord {
        TurnRecord { prompt: prompt.into(), answer: answer.into(), status: TurnStatus::Answered, model: String::new(), route: None, usage: None, elapsed_ms: 0, ended_ms: 0, tool_calls: 0 }
    }

    #[test]
    fn the_text_lists_only_sources_with_something_in_them() {
        let b = breakdown(&[turn(&"a".repeat(400), &"b".repeat(400))], &[], None);
        let text = breakdown_text(&b, AutoCompact::At(90), Style::off());
        assert!(text.contains("your messages") && text.contains("answers"), "{text}");
        assert!(!text.contains("waiting") && !text.contains("memory  "), "{text}");
        assert!(text.contains("50%") && text.contains("heaviest turn: 1") && text.contains("compacts itself at 90%"), "{text}");
        assert!(breakdown_text(&Breakdown::default(), AutoCompact::Off, Style::off()).contains("auto-compaction is off"));
    }
}
