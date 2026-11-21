//! `/replay [n]`: read a saved session back, turn by turn, without resuming it: what was
//! asked, which model answered, the tools it used and the answer. Redacted like an export,
//! because a replay is what people paste into a chat.

use crate::domain::turn_record::TurnRecord;
use jev_shared::domain::redact::redact;

/// How many tool summaries one turn shows before "and N more".
const TOOLS_SHOWN: usize = 8;
/// How much of one answer a replay shows, in characters.
const ANSWER_CHARS: usize = 1200;

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// One turn as lines of text.
fn turn_lines(n: usize, total: usize, t: &TurnRecord) -> Vec<String> {
    let mut lines = vec![format!("── {n}/{total} ── {} · {}{}", t.provider, t.model, if t.ok { "" } else { "  (did not finish)" })];
    lines.extend(redact(t.prompt.trim()).lines().map(|l| format!("  ❯ {l}")));
    for tool in t.tools.iter().take(TOOLS_SHOWN) {
        lines.push(format!("    ⎿ {}", redact(tool)));
    }
    if t.tools.len() > TOOLS_SHOWN {
        lines.push(format!("    ⎿ and {} more tool calls", t.tools.len() - TOOLS_SHOWN));
    }
    let answer = redact(t.reply.trim());
    if !answer.is_empty() {
        lines.extend(shorten(&answer, ANSWER_CHARS).lines().map(|l| format!("  {l}")));
    }
    lines
}

/// The whole session, or `None` when it has no turns.
pub fn replay_text(title: &str, turns: &[TurnRecord]) -> Option<String> {
    if turns.is_empty() {
        return None;
    }
    let mut lines = vec![format!("  Replay of \"{}\": {} turns (read only; /resume continues it)", shorten(&redact(title), 60), turns.len())];
    for (i, t) in turns.iter().enumerate() {
        lines.extend(turn_lines(i + 1, turns.len(), t));
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_backend::ProviderId;

    fn turn(prompt: &str, reply: &str, tools: usize, ok: bool) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            provider: ProviderId::Claude,
            model: "claude-sonnet-5-5".into(),
            reply: reply.into(),
            tools: (0..tools).map(|i| format!("Edit src/f{i}.rs")).collect(),
            ok,
        }
    }

    #[test]
    fn an_empty_session_has_nothing_to_replay() {
        assert_eq!(replay_text("x", &[]), None);
    }

    #[test]
    fn a_replay_shows_each_turn_with_model_tools_and_answer() {
        let text = replay_text("fix", &[turn("fix it\nplease", "Done.", 2, true), turn("again", "", 0, false)]).unwrap();
        assert!(text.contains("2 turns") && text.contains("── 1/2 ── claude · claude-sonnet-5-5\n"), "{text}");
        assert!(text.contains("  ❯ fix it\n  ❯ please\n    ⎿ Edit src/f0.rs\n    ⎿ Edit src/f1.rs\n  Done."), "{text}");
        assert!(text.contains("── 2/2 ── claude · claude-sonnet-5-5  (did not finish)"), "{text}");
    }

    #[test]
    fn many_tools_and_long_answers_are_cut_and_secrets_are_masked() {
        let long = format!("API_KEY=hunter2hunter2 {}", "x".repeat(2000));
        let text = replay_text("t", &[turn("go", &long, 11, true)]).unwrap();
        assert!(text.contains("and 3 more tool calls") && text.contains("…"), "{text}");
        assert!(!text.contains("hunter2") && text.contains("[REDACTED]"), "{text}");
    }
}
