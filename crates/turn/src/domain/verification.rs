//! What running a check (a compile check, the project's tests) after a turn showed. A check
//! is evidence; whether the turn is accepted is judged from it in `jev_shared::domain::acceptance`.

/// What running the check showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// The tests ran and passed.
    Passed,
    /// The tests ran and failed; the end of their output.
    Failed { output: String },
    /// The tests did not give an answer (none found, could not start, timed out, stopped):
    /// neither a pass nor a failure of the work.
    Inconclusive { why: String },
}

impl Verification {
    /// The verdict worth recording: only a pass or a failure decided anything.
    pub fn verdict(&self) -> Option<jev_shared::domain::audit::Verdict> {
        match self {
            Self::Passed => Some(jev_shared::domain::audit::Verdict::Passed),
            Self::Failed { .. } => Some(jev_shared::domain::audit::Verdict::Failed),
            Self::Inconclusive { .. } => None,
        }
    }

    /// One word for logs and the scoreboard.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed { .. } => "failed",
            Self::Inconclusive { .. } => "inconclusive",
        }
    }

    /// The line the user is told, without the test output.
    pub fn summary(&self) -> String {
        match self {
            Self::Passed => "[verify] the check passed".to_string(),
            Self::Failed { .. } => "[verify] the check failed; the work is not accepted".to_string(),
            Self::Inconclusive { why } => format!("[verify] not checked: {why}"),
        }
    }
}

/// Most characters of a failed check's output the next turn is told.
pub const FAILURE_CONTEXT_CHARS: usize = 2_000;

/// What the next turn is told after a check failed: the command and the end of its output.
pub fn failure_context(command: &str, output: &str) -> String {
    let chars: Vec<char> = output.trim_end().chars().collect();
    let cut = chars.len().saturating_sub(FAILURE_CONTEXT_CHARS);
    let tail: String = chars[cut..].iter().collect();
    let shown = if cut > 0 { format!("[earlier output not shown]\n{tail}") } else { tail };
    format!("[verification failed] the check `{command}` failed after your previous turn's changes:\n{shown}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::domain::audit::Verdict;

    #[test]
    fn tags_and_summaries_say_what_happened() {
        assert_eq!(Verification::Passed.tag(), "passed");
        assert_eq!(Verification::Failed { output: "boom".into() }.tag(), "failed");
        let none = Verification::Inconclusive { why: "no test command".into() };
        assert_eq!(none.tag(), "inconclusive");
        assert_eq!(none.summary(), "[verify] not checked: no test command");
        assert!(!Verification::Failed { output: "boom".into() }.summary().contains("boom"));
        assert_eq!(Verification::Passed.verdict(), Some(Verdict::Passed));
        assert_eq!(Verification::Failed { output: String::new() }.verdict(), Some(Verdict::Failed));
        assert_eq!(none.verdict(), None);
    }

    #[test]
    fn a_failure_is_told_with_the_end_of_its_output() {
        let c = failure_context("cargo check", "error[E0308]\nmismatched types");
        assert!(c.starts_with("[verification failed]") && c.contains("`cargo check`") && c.ends_with("mismatched types"), "{c}");
        let long = failure_context("t", &format!("{}END", "x".repeat(5_000)));
        assert!(long.contains("[earlier output not shown]") && long.ends_with("END"));
        assert!(long.chars().count() < FAILURE_CONTEXT_CHARS + 200);
    }
}
