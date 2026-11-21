//! The subagent visuals: panel, footer cue, nested transcript, conflicts, result card and
//! timeline.
//!
//! The worker types live here; each visual is a child module.

use crate::render::Style;

/// Where a worker is. Ordered the way the panel sorts: what needs you first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkerState {
    WaitingApproval,
    Running,
    Queued,
    Failed,
    Cancelled,
    Done,
}

impl WorkerState {
    pub fn word(self) -> &'static str {
        match self {
            Self::WaitingApproval => "waiting",
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Done => "done",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::WaitingApproval => "?",
            Self::Running => "▶",
            Self::Queued => "·",
            Self::Failed => "✗",
            Self::Cancelled => "−",
            Self::Done => "✓",
        }
    }

    fn live(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval | Self::Queued)
    }

    fn paint(self, s: Style, text: &str) -> String {
        match self {
            Self::Done => s.green(text),
            Self::Failed => s.red(text),
            Self::WaitingApproval => s.yellow(text),
            Self::Running => s.cyan(text),
            Self::Queued | Self::Cancelled => s.dim(text),
        }
    }
}

/// One worker as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub task: String,
    pub state: WorkerState,
    pub elapsed_ms: u64,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
    pub files: Vec<String>,
    /// "editing src/parser.rs": what it is doing right now.
    pub last_action: Option<String>,
    /// 0 for a worker of the planner, 1 for one a worker spawned.
    pub depth: usize,
}

impl WorkerInfo {
    pub fn new(id: &str, provider: &str, model: &str, task: &str, state: WorkerState) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            model: model.into(),
            task: task.into(),
            state,
            elapsed_ms: 0,
            tokens: 0,
            cost_usd: None,
            files: vec![],
            last_action: None,
            depth: 0,
        }
    }
}

pub mod conflicts;
pub mod panel;
pub mod result;
pub mod timeline;
pub mod transcript;
#[cfg(test)]
mod tests;
