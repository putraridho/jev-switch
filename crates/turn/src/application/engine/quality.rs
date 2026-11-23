//! After a turn: check its changes when asked to, judge whether the work was accepted, and
//! keep the outcome where the next turn and the router can see it.

use std::sync::Arc;

use jev_approval::domain::mode::Mode;
use jev_orchestration::application::ports::{Checks, QuickChecks};
use jev_routing::domain::track_record::TrackRecords;
use jev_session::domain::turn_status::TurnStatus;
use jev_shared::domain::acceptance::{judge, rework_of, Evidence, Prior, Quality};
use jev_shared::Usage;
use crate::application::cancel::Cancel;
use crate::application::ports::Ui;
use crate::application::turn_report::TurnReport;
use crate::application::verify::verify;
use crate::domain::outcomes::{spent_tokens, Outcomes};
use crate::domain::verification::{failure_context, Verification};
use crate::domain::verify_mode::VerifyMode;
use super::Engine;

/// What the engine keeps to check, judge and learn from turns.
pub(super) struct QualityState {
    /// `/verify`: off until asked for, because a check takes time and runs on the person's machine.
    pub(super) verify: VerifyMode,
    /// Runs the check; `None` verifies nothing.
    pub(super) checks: Option<Arc<dyn Checks>>,
    /// Finds the cheap check of the project for `auto`.
    pub(super) quick: Option<Arc<dyn QuickChecks>>,
    /// Let accepted outcomes reorder equal-tier models (`JEV_LEARNED_ROUTING`).
    pub(super) learn: bool,
    /// What the past turns, from the audit log and this session, came to.
    pub(super) outcomes: Outcomes,
    /// The part of `outcomes` the router reads.
    pub(super) records: TrackRecords,
    /// The failed check of the last turn, to tell the next one.
    pub(super) pending_failure: Option<String>,
    /// What the last turn left behind, to spot rework.
    pub(super) prior: Option<Prior>,
    /// Of the turn in progress: tokens over all its attempts, the task type, and the failed
    /// tool calls of the attempt that ended it.
    pub(super) spent: u64,
    pub(super) task: String,
    pub(super) tool_errors: u32,
}

impl Default for QualityState {
    fn default() -> Self {
        Self {
            verify: VerifyMode::Off,
            checks: None,
            quick: None,
            learn: true,
            outcomes: Outcomes::default(),
            records: TrackRecords::default(),
            pending_failure: None,
            prior: None,
            spent: 0,
            task: String::new(),
            tool_errors: 0,
        }
    }
}

impl QualityState {
    pub(super) fn begin_turn(&mut self) {
        self.spent = 0;
        self.task.clear();
        self.tool_errors = 0;
    }

    pub(super) fn add_spend(&mut self, usage: &Usage) {
        self.spent = self.spent.saturating_add(spent_tokens(usage));
    }
}

impl Engine {
    /// The prompt as the model gets it: after a failed check, what failed comes first.
    pub(super) fn with_check_context(&mut self, prompt: &str) -> String {
        match self.quality.pending_failure.take() {
            Some(context) => format!("{context}\n\n{prompt}"),
            None => prompt.to_string(),
        }
    }

    /// A turn that never started a model did not use what it was told: keep it for the next.
    pub(super) fn keep_check_context(&mut self, sent: &str, prompt: &str) {
        if sent != prompt && self.quality.pending_failure.is_none() {
            self.quality.pending_failure = sent.strip_suffix(prompt).map(|c| c.trim_end().to_string());
        }
    }

    /// The command that checks this turn, `None` when nothing is to run (and why, for the person).
    fn check_command(&self) -> Result<String, String> {
        match &self.quality.verify {
            VerifyMode::Off => Err(String::new()),
            VerifyMode::Command(c) => Ok(c.clone()),
            VerifyMode::Auto => self
                .quality
                .quick
                .as_ref()
                .and_then(|q| q.quick_command(&self.dir))
                .ok_or_else(|| "no cheap check found for this folder".to_string()),
        }
    }

    /// When verification is on and the turn answered and changed files (never in plan mode),
    /// run the check, tell the user, count the verdict against the provider that answered, and
    /// keep a failure to tell the next turn. Returns how long the check took.
    pub(super) async fn verify_report(&mut self, report: &mut TurnReport, ui: &mut dyn Ui, cancel: &Cancel) -> u64 {
        self.quality.pending_failure = None;
        if self.mode == Mode::Plan || report.status != TurnStatus::Answered || report.changes.is_none() || cancel.is_cancelled() {
            return 0;
        }
        let Some(checks) = self.quality.checks.clone() else { return 0 };
        let command = match self.check_command() {
            Ok(command) => command,
            Err(why) if why.is_empty() => return 0,
            Err(why) => {
                let v = Verification::Inconclusive { why };
                ui.note(&v.summary());
                report.verification = Some(v);
                return 0;
            }
        };
        ui.note(&format!("[verify] running `{command}`"));
        let started = (self.now)();
        let verification = verify(checks, &self.dir, &command, cancel).await;
        let took = (self.now)().saturating_sub(started);
        ui.note(&verification.summary());
        if let Some(provider) = report.provider {
            self.guards.scoreboard.record_verification(provider, &verification);
        }
        if let Verification::Failed { output } = &verification {
            self.quality.pending_failure = Some(failure_context(&command, output));
        }
        report.verification = Some(verification);
        took
    }

    /// Judge the finished turn from what was observed, and remember it: in the report (for the
    /// audit log), for spotting rework next turn, and in the outcomes the router reads.
    pub(super) fn grade(&mut self, report: &mut TurnReport, verify_ms: u64) {
        let files: Vec<String> = report.changes.as_ref().map(|c| c.files.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
        let check = report.verification.as_ref().and_then(Verification::verdict);
        let acceptance = judge(&Evidence { answered: report.status == TurnStatus::Answered, changed_files: !files.is_empty(), check, tool_errors: self.quality.tool_errors });
        let rework = rework_of(self.quality.prior.as_ref(), &files);
        report.quality = Quality {
            task: self.quality.task.clone(),
            acceptance,
            verify_ms,
            retries: report.attempts.saturating_sub(1),
            rework,
            spent_tokens: self.quality.spent,
        };
        self.quality.prior = Some(Prior { files, failed_check: matches!(check, Some(jev_shared::domain::audit::Verdict::Failed)), reverted: false });
        if let (Some(provider), Some(model)) = (report.provider, &report.model) {
            let key = format!("{provider} · {model}");
            self.quality.outcomes.add_turn(&self.dir.display().to_string(), &key, report.status == TurnStatus::Answered, &report.quality);
            self.quality.records = self.quality.outcomes.records();
        }
    }
}
