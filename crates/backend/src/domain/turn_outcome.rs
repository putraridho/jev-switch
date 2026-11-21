//! A successful turn.

use super::plan_usage::PlanUsage;
use jev_shared::Usage;

/// A successful turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    pub text: String,
    pub session_id: Option<String>,
    pub usage: Usage,
    pub plan: Option<PlanUsage>,
}
