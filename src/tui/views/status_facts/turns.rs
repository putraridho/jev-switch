//! Owns the facts of a finished turn: the changed files, the one-line summary and the exit code.
//! Must not print.

use crate::changes::domain::change_set::ChangeSummary;
use crate::changes::domain::file_change::ChangeStatus;
use crate::providers::application::provider_registry::Runtime;
use crate::turn::application::turn_report::TurnReport;
use crate::turn::domain::turn_status::TurnStatus;
use crate::tui::render::{FileChange, SummaryFacts};

fn change_letter(s: ChangeStatus) -> char {
    match s {
        ChangeStatus::Added => 'A',
        ChangeStatus::Modified => 'M',
        ChangeStatus::Deleted => 'D',
        ChangeStatus::Renamed => 'R',
        ChangeStatus::ModifiedUntracked => 'U',
    }
}

/// The changed files as the formatter wants them.
pub fn file_changes(summary: &ChangeSummary) -> Vec<FileChange> {
    summary
        .files
        .iter()
        .map(|f| FileChange { status: change_letter(f.status), path: f.path.clone(), added: f.added, removed: f.removed })
        .collect()
}

/// The facts of the one-line summary after a turn. `elapsed_ms` is the wall time the
/// caller measured; the plan percentage is the last one the answering provider reported.
pub fn summary_facts(report: &TurnReport, elapsed_ms: u64, rt: &Runtime) -> SummaryFacts {
    let usage = report.usage.unwrap_or_default();
    SummaryFacts {
        ok: report.status == TurnStatus::Answered,
        cancelled: report.status == TurnStatus::Cancelled,
        provider: report.provider.map(|p| p.to_string()).unwrap_or_default(),
        model: report.model.clone().unwrap_or_default(),
        elapsed_ms,
        in_tokens: usage.input_tokens,
        cached_tokens: usage.cached_input_tokens,
        out_tokens: usage.output_tokens,
        plan: report.provider.and_then(|p| Some((p.to_string(), rt.plan_utilization(p)?))),
        files_changed: report.changes.as_ref().map(|c| {
            let (added, removed) = c.totals();
            (c.files.len(), added, removed)
        }),
    }
}

/// The process exit code of a one-shot turn: 0 answered, 130 stopped (the shell's
/// convention for Ctrl-C), 1 for anything else.
pub fn exit_code(status: TurnStatus) -> i32 {
    match status {
        TurnStatus::Answered => 0,
        TurnStatus::Cancelled => 130,
        TurnStatus::NoRoute | TurnStatus::Failed => 1,
    }
}
