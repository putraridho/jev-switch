//! Context intelligence: what fills the window (for `/context`) and when the session
//! should compact itself. Pure: sizes in, decisions out. Token counts here are estimates
//! (four characters a token); the real total comes from the model's own usage report.

use crate::domain::session_log::TurnRecord;

/// Characters per token, the usual rough figure.
const CHARS_PER_TOKEN: usize = 4;

/// An estimate of the tokens in `text`.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() / CHARS_PER_TOKEN) as u64
}

/// Where the session's text sits, in estimated tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakdown {
    /// What the person typed (and the notes saved with it in the log).
    pub prompts: u64,
    pub answers: u64,
    /// Notes waiting for the next message (`!command` output, a summary, the repo map).
    pub waiting: u64,
    /// Project memory, sent with the first message of a session.
    pub memory: u64,
    /// The largest single turn, as `(number from 1, tokens)`: usually what to `/compact` away.
    pub heaviest_turn: Option<(usize, u64)>,
}

impl Breakdown {
    pub fn total(&self) -> u64 {
        self.prompts + self.answers + self.waiting + self.memory
    }
}

pub fn breakdown(turns: &[TurnRecord], waiting: &[String], memory: Option<&str>) -> Breakdown {
    let mut b = Breakdown { waiting: waiting.iter().map(|n| estimate_tokens(n)).sum(), memory: memory.map(estimate_tokens).unwrap_or_default(), ..Breakdown::default() };
    for (i, t) in turns.iter().enumerate() {
        let (p, a) = (estimate_tokens(&t.prompt), estimate_tokens(&t.answer));
        b.prompts += p;
        b.answers += a;
        if b.heaviest_turn.map_or(p + a > 0, |(_, best)| p + a > best) {
            b.heaviest_turn = Some((i + 1, p + a));
        }
    }
    b
}

/// When a session compacts itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCompact {
    Off,
    /// At this percent of the window or more.
    At(u8),
}

/// The percent used when nothing is configured.
pub const DEFAULT_AT: u8 = 90;

/// The least turns a session needs before it compacts itself: a summary of one turn
/// saves nothing.
const MIN_TURNS: usize = 2;

impl AutoCompact {
    /// `JEV_AUTOCOMPACT`: `off`, or a percent from 50 to 99; anything else is the default.
    pub fn parse(setting: Option<&str>) -> Self {
        match setting.map(str::trim) {
            Some("off" | "0" | "false" | "no") => Self::Off,
            Some(p) => match p.trim_end_matches('%').parse::<u8>() {
                Ok(n) if (50..=99).contains(&n) => Self::At(n),
                _ => Self::At(DEFAULT_AT),
            },
            None => Self::At(DEFAULT_AT),
        }
    }

    /// Whether the next message should be preceded by a compaction.
    pub fn due(self, used_percent: Option<u8>, turns: usize) -> bool {
        match (self, used_percent) {
            (Self::At(limit), Some(used)) => used >= limit && turns >= MIN_TURNS,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::turn_status::TurnStatus;

    fn turn(prompt: &str, answer: &str) -> TurnRecord {
        TurnRecord { prompt: prompt.into(), answer: answer.into(), status: TurnStatus::Answered, model: String::new(), route: None, usage: None, elapsed_ms: 0, ended_ms: 0, tool_calls: 0, verdict: None }
    }

    #[test]
    fn the_breakdown_splits_prompts_answers_notes_and_memory_and_names_the_heaviest_turn() {
        let turns = [turn(&"a".repeat(40), &"b".repeat(80)), turn("hi", &"c".repeat(400))];
        let b = breakdown(&turns, &["x".repeat(120)], Some(&"m".repeat(16)));
        assert_eq!((b.prompts, b.answers, b.waiting, b.memory), (10, 20 + 100, 30, 4));
        assert_eq!(b.heaviest_turn, Some((2, 100)));
        assert_eq!(b.total(), 10 + 120 + 30 + 4);
        assert_eq!(breakdown(&[], &[], None), Breakdown::default());
    }

    #[test]
    fn the_setting_reads_off_a_percent_or_falls_back() {
        assert_eq!(AutoCompact::parse(None), AutoCompact::At(90));
        assert_eq!(AutoCompact::parse(Some("off")), AutoCompact::Off);
        assert_eq!(AutoCompact::parse(Some("75%")), AutoCompact::At(75));
        assert_eq!(AutoCompact::parse(Some("10")), AutoCompact::At(90), "too low to be useful");
        assert_eq!(AutoCompact::parse(Some("soon")), AutoCompact::At(90));
    }

    #[test]
    fn compaction_is_due_only_past_the_limit_with_enough_turns_and_a_known_window() {
        let on = AutoCompact::At(90);
        assert!(on.due(Some(90), 5));
        assert!(!on.due(Some(89), 5));
        assert!(!on.due(None, 5));
        assert!(!on.due(Some(99), 1));
        assert!(!AutoCompact::Off.due(Some(99), 9));
    }
}
