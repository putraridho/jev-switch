//! Whether a turn's work was checked, and whether the check held: an answer is text, an
//! accepted turn is work that passed the project's own tests.

/// What running the project's tests after a turn showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acceptance {
    /// The tests ran and passed.
    Passed,
    /// The tests ran and failed; the end of their output.
    Failed { output: String },
    /// The tests did not give an answer (none found, could not start, timed out, stopped):
    /// neither a pass nor a failure of the work.
    Inconclusive { why: String },
}

impl Acceptance {
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
            Self::Passed => "[verify] the project's tests passed".to_string(),
            Self::Failed { .. } => "[verify] the project's tests failed; the answer is not accepted".to_string(),
            Self::Inconclusive { why } => format!("[verify] not checked: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::domain::audit::Verdict;

    #[test]
    fn tags_and_summaries_say_what_happened() {
        assert_eq!(Acceptance::Passed.tag(), "passed");
        assert_eq!(Acceptance::Failed { output: "boom".into() }.tag(), "failed");
        let none = Acceptance::Inconclusive { why: "no test command".into() };
        assert_eq!(none.tag(), "inconclusive");
        assert_eq!(none.summary(), "[verify] not checked: no test command");
        assert!(!Acceptance::Failed { output: "boom".into() }.summary().contains("boom"));
        assert_eq!(Acceptance::Passed.verdict(), Some(Verdict::Passed));
        assert_eq!(Acceptance::Failed { output: String::new() }.verdict(), Some(Verdict::Failed));
        assert_eq!(none.verdict(), None);
    }
}
