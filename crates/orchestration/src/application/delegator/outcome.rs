//! How one attempt, and one whole delegation, ended before anything lands in the main tree.

use jev_backend::TurnError;
use jev_catalog::domain::model_table::ModelSpec;
use crate::domain::delegation::{DelegateResult, ReviewReason};


/// One worker attempt that failed, in the words the planner and the log need.
pub(super) struct AttemptFailure {
    pub(super) id: String,
    pub(super) spec: &'static ModelSpec,
    pub(super) err: TurnError,
}

/// What the tests said about an editing worker's work, before it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The tests passed: it may merge on its own if it applies cleanly.
    Passed { command: String },
    Review(ReviewReason),
}

impl Verdict {
    pub(super) fn tag(&self) -> &'static str {
        match self {
            Self::Passed { .. } => "tests_passed",
            Self::Review(r) => r.tag(),
        }
    }
}

/// A delegation that ran to its end. Nothing of it has touched the main tree yet.
pub(super) enum Finished {
    /// Nothing to land: a failure, a read-only report, or no changes.
    Done(Box<DelegateResult>),
    /// An editing worker's changes, waiting for `Delegator::settle`.
    Pending(Box<Pending>),
}

impl Finished {
    pub(super) fn done(result: DelegateResult) -> Self {
        Self::Done(Box::new(result))
    }

    pub(super) fn with_retries(mut self, retries: Vec<String>) -> Self {
        match &mut self {
            Self::Done(result) => result.retries = retries,
            Self::Pending(pending) => pending.result.retries = retries,
        }
        self
    }
}

/// An editing worker's finished work. `result` is complete except for how it lands
/// (`landing`, and for review the diff page), which `Delegator::settle` fills in.
pub(super) struct Pending {
    pub(super) result: DelegateResult,
    pub(super) patch: String,
    pub(super) verdict: Verdict,
}

/// How one attempt ended without a result.
pub(super) enum AttemptOutcome {
    /// Nothing more to try (no worker, no worktree, unreadable changes).
    Stop(String),
    /// The worker failed; [`Runtime::react`] says whether another model may try.
    Failed(AttemptFailure),
}
