//! Whether a user turn's work was accepted: judged from what could be observed, never from
//! the fact that a model answered. Text is not evidence; a passing check is.

use crate::domain::audit::Verdict;

/// Why a turn's work was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The project's check ran after the turn and failed.
    VerificationFailed,
    /// The person undid the turn's changes afterwards.
    Reverted,
}

/// The verdict on one user turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Acceptance {
    /// The work was checked and held up.
    Accepted,
    /// The work was checked or taken back and did not hold up.
    Rejected(Rejection),
    /// Nothing observable says either way: a turn that only answered, changed nothing, was
    /// not checked, or did not end in an answer.
    #[default]
    Unverified,
}

impl Acceptance {
    /// One word for logs and tables.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected(Rejection::VerificationFailed) => "rejected (check failed)",
            Self::Rejected(Rejection::Reverted) => "rejected (reverted)",
            Self::Unverified => "unverified",
        }
    }
}

/// Everything observed about a turn that bears on whether it was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    /// The turn ended in an answer.
    pub answered: bool,
    /// The turn changed files in the working tree.
    pub changed_files: bool,
    /// What the check said, when one ran and decided.
    pub check: Option<Verdict>,
    /// Tool calls of the turn that failed.
    pub tool_errors: u32,
}

/// Judge a turn. Only a check that ran and passed accepts work; a failed check rejects it.
/// Tool errors never accept or reject by themselves (a failed search is routine), and a turn
/// without a decisive check stays unverified however confident its text was.
pub fn judge(e: &Evidence) -> Acceptance {
    if !e.answered {
        return Acceptance::Unverified;
    }
    match e.check {
        Some(Verdict::Failed) => Acceptance::Rejected(Rejection::VerificationFailed),
        Some(Verdict::Passed) if e.changed_files => Acceptance::Accepted,
        _ => Acceptance::Unverified,
    }
}

/// Why a turn redid an earlier turn's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rework {
    #[default]
    None,
    /// The turn before failed its check and this one edited the same files again.
    AfterFailedCheck,
    /// The person undid the turn before and this one edited the same files again.
    AfterRevert,
}

/// What the turn before left behind, for spotting rework.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prior {
    pub files: Vec<String>,
    pub failed_check: bool,
    pub reverted: bool,
}

/// Whether a turn that changed `files` redid the work of the `prior` turn: only the turn
/// right after one that failed its check or was undone counts, and only on a shared file.
pub fn rework_of(prior: Option<&Prior>, files: &[String]) -> Rework {
    let Some(prior) = prior else { return Rework::None };
    if !files.iter().any(|f| prior.files.contains(f)) {
        return Rework::None;
    }
    if prior.reverted {
        Rework::AfterRevert
    } else if prior.failed_check {
        Rework::AfterFailedCheck
    } else {
        Rework::None
    }
}

/// What a finished turn adds to the audit log beyond its tokens and time. Every field has an
/// empty default, which is what a line written before these existed reads as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Quality {
    /// The task type the router judged ("feature", "security", ...); empty when unknown.
    pub task: String,
    pub acceptance: Acceptance,
    /// How long the check took, in ms; 0 when none ran.
    pub verify_ms: u64,
    /// Models started beyond the first for this turn (failovers).
    pub retries: u32,
    pub rework: Rework,
    /// Tokens the turn consumed over all its attempts, the cached share left out; 0 when the
    /// CLIs did not report them.
    pub spent_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(answered: bool, changed: bool, check: Option<Verdict>) -> Evidence {
        Evidence { answered, changed_files: changed, check, tool_errors: 0 }
    }

    #[test]
    fn an_answer_alone_is_never_accepted() {
        assert_eq!(judge(&ev(true, false, None)), Acceptance::Unverified);
        assert_eq!(judge(&ev(true, true, None)), Acceptance::Unverified);
        assert_eq!(judge(&ev(true, false, Some(Verdict::Passed))), Acceptance::Unverified);
    }

    #[test]
    fn a_check_decides() {
        assert_eq!(judge(&ev(true, true, Some(Verdict::Passed))), Acceptance::Accepted);
        assert_eq!(judge(&ev(true, true, Some(Verdict::Failed))), Acceptance::Rejected(Rejection::VerificationFailed));
        assert_eq!(judge(&ev(false, true, Some(Verdict::Passed))), Acceptance::Unverified);
    }

    #[test]
    fn tool_errors_do_not_decide() {
        let mut e = ev(true, true, Some(Verdict::Passed));
        e.tool_errors = 3;
        assert_eq!(judge(&e), Acceptance::Accepted);
        e.check = None;
        assert_eq!(judge(&e), Acceptance::Unverified);
    }

    #[test]
    fn rework_needs_a_bad_turn_before_and_a_shared_file() {
        let files = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let failed = Prior { files: files(&["a.rs"]), failed_check: true, reverted: false };
        assert_eq!(rework_of(Some(&failed), &files(&["a.rs", "b.rs"])), Rework::AfterFailedCheck);
        assert_eq!(rework_of(Some(&failed), &files(&["b.rs"])), Rework::None);
        assert_eq!(rework_of(None, &files(&["a.rs"])), Rework::None);
        let undone = Prior { reverted: true, ..failed.clone() };
        assert_eq!(rework_of(Some(&undone), &files(&["a.rs"])), Rework::AfterRevert);
        let fine = Prior { failed_check: false, ..failed };
        assert_eq!(rework_of(Some(&fine), &files(&["a.rs"])), Rework::None);
    }

    #[test]
    fn the_default_is_unverified() {
        assert_eq!(Quality::default().acceptance, Acceptance::Unverified);
        assert_eq!(Acceptance::default().tag(), "unverified");
    }
}
