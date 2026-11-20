//! Look away from a long turn: the terminal's title bar says what the agent is doing and a
//! bell rings when it needs you or is done.

use std::time::Duration;

/// What the agent is doing, as the title bar says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Running,
    /// Waiting for an approval or an answer.
    Waiting,
    Done,
    Failed,
}

impl Activity {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "ready",
            Self::Running => "working",
            Self::Waiting => "needs you",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn mark(self) -> &'static str {
        match self {
            Self::Idle => "○",
            Self::Running => "●",
            Self::Waiting => "?",
            Self::Done => "✓",
            Self::Failed => "✗",
        }
    }
}

/// The OSC sequence that sets the window title: `● working · session name · jev-code`.
/// Control characters in the session name are dropped so a name can never end the sequence.
pub fn title_sequence(activity: Activity, session: &str) -> String {
    let name: String = session.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    let tail = if name.is_empty() { String::new() } else { format!(" · {name}") };
    format!("\x1b]0;{} {}{tail} · jev-code\x07", activity.mark(), activity.label())
}

pub const BELL: &str = "\x07";

/// A turn shorter than this does not ring: you were still looking.
pub const BELL_AFTER: Duration = Duration::from_secs(20);

/// Whether moving to `to` after `elapsed` should ring the bell: always when the agent needs
/// you, and when a turn that took a while is over.
pub fn should_ring(to: Activity, elapsed: Duration) -> bool {
    match to {
        Activity::Waiting => true,
        Activity::Done | Activity::Failed => elapsed >= BELL_AFTER,
        Activity::Idle | Activity::Running => false,
    }
}

/// What to write to the terminal when the activity changes: the title, and the bell when it
/// is worth one.
pub fn on_change(to: Activity, session: &str, elapsed: Duration) -> String {
    let bell = if should_ring(to, elapsed) { BELL } else { "" };
    format!("{}{bell}", title_sequence(to, session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_says_what_is_happening() {
        assert_eq!(title_sequence(Activity::Running, "parser fix"), "\x1b]0;● working · parser fix · jev-code\x07");
        assert_eq!(title_sequence(Activity::Done, ""), "\x1b]0;✓ done · jev-code\x07");
        assert_eq!(title_sequence(Activity::Waiting, "x"), "\x1b]0;? needs you · x · jev-code\x07");
    }

    #[test]
    fn a_session_name_cannot_inject_escapes() {
        let t = title_sequence(Activity::Idle, "a\x07\x1b]0;evil\x07b");
        assert_eq!(t.matches('\x07').count(), 1, "only the terminator: {t:?}");
        assert_eq!(t.matches('\x1b').count(), 1);
    }

    #[test]
    fn bell_policy() {
        let long = Duration::from_secs(45);
        let short = Duration::from_secs(3);
        assert!(should_ring(Activity::Waiting, short));
        assert!(should_ring(Activity::Done, long));
        assert!(should_ring(Activity::Failed, long));
        assert!(!should_ring(Activity::Done, short));
        assert!(!should_ring(Activity::Running, long));
        assert!(on_change(Activity::Done, "s", long).ends_with("\x07\x07"));
        assert_eq!(on_change(Activity::Done, "s", short).matches('\x07').count(), 1);
    }
}
