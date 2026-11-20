//! What a turn came to.

use crate::backend::{ProviderId, Usage};
use crate::turn::domain::turn_status::TurnStatus;
use crate::changes::infrastructure::git_working_tree::ChangeSummary;
#[derive(Debug, Clone)]
pub struct TurnReport {
    pub status: TurnStatus,
    /// The answer; empty unless `status` is [`TurnStatus::Answered`].
    pub text: String,
    /// What the user was told when the turn did not end in an answer.
    pub message: String,
    /// The provider and model that answered, or that failed last.
    pub provider: Option<ProviderId>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    /// Models started for this turn.
    pub attempts: u32,
    /// Tool calls the model made in the attempt that ended the turn.
    pub tool_calls: u32,
    /// What the turn changed in the working tree. `None` when the folder is not a git
    /// repository, git is missing or failed, no model was started, or the turn changed
    /// nothing (a summary kept here always lists at least one file).
    pub changes: Option<ChangeSummary>,
}

impl TurnReport {
    pub(super) fn ended(status: TurnStatus, message: String, attempts: u32) -> Self {
        Self { status, text: String::new(), message, provider: None, model: None, usage: None, attempts, tool_calls: 0, changes: None }
    }
}
