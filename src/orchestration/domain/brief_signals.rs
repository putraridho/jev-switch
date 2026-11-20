//! What Jev said about a brief, reduced to what choosing a worker needs, and the two rules
//! that read it: which task type picks the chain, and the lowest tier a worker may run on.

use crate::catalog::domain::model_table::{TaskType, Tier};
use crate::judgment::application::judge::{CODE_EDIT_P, CODE_SECURITY_P};

/// What Jev said about a brief, reduced to what choosing a worker needs.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefSignals {
    pub task: TaskType,
    /// P(the brief is about security), independent of keywords.
    pub security_p: f64,
    /// P(the brief changes files or runs commands).
    pub edit_p: f64,
    /// The planner says no files change. Only a hint: it lowers the floor to Light only
    /// when Jev also says the brief is not an edit.
    pub read_only: bool,
}

impl BriefSignals {
    /// A brief Jev judged to be an edit of the given task type, with no security suspicion.
    #[cfg(test)]
    pub fn editing(task: TaskType) -> Self {
        Self { task, security_p: 0.0, edit_p: 0.9, read_only: false }
    }

    /// Jev was unavailable. A brief nobody judged could be security work, which has hard
    /// vendor rules, so it is treated as security (never DeepSeek/Grok) and as an edit.
    pub fn unjudged() -> Self {
        Self { task: TaskType::Security, security_p: 1.0, edit_p: 1.0, read_only: false }
    }
}

/// The task type that picks the chain. Security is a hard rule that overrides Jev's task
/// label, like in routing, but judged by Jev's `security_p` only: the keyword lists
/// are not used for workers because a brief is full of incidental words ("no unsafe
/// code") that tagged every worker security-sensitive.
pub fn worker_task(s: &BriefSignals) -> TaskType {
    if s.task == TaskType::Security || s.security_p >= CODE_SECURITY_P {
        TaskType::Security
    } else {
        s.task
    }
}

/// The lowest tier this worker may run on. Work that writes code is never below
/// Everyday; Light is for lookups the planner calls read-only AND Jev agrees do not edit.
pub fn worker_floor(s: &BriefSignals) -> Tier {
    if s.read_only && s.edit_p < CODE_EDIT_P {
        Tier::Light
    } else {
        Tier::Everyday
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jev_unavailable_is_treated_as_security_and_as_an_edit() {
        let u = BriefSignals::unjudged();
        assert_eq!(worker_task(&u), TaskType::Security);
        assert_eq!(worker_floor(&u), Tier::Everyday);
    }
}
