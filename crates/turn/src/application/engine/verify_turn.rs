//! After a turn: check its changes with the project's tests and record the verdict.

use jev_session::domain::turn_status::TurnStatus;
use crate::application::cancel::Cancel;
use crate::application::ports::Ui;
use crate::application::turn_report::TurnReport;
use crate::application::verify::verify;
use super::Engine;

impl Engine {
    /// When verification is on and the turn answered and changed files, run the project's
    /// tests, tell the user the verdict, and count it against the provider that answered.
    /// Anything else (off, cancelled, failed, nothing changed) is left alone.
    pub(super) async fn verify_report(&mut self, report: &mut TurnReport, ui: &mut dyn Ui, cancel: &Cancel) {
        if !self.verify || report.status != TurnStatus::Answered || report.changes.is_none() || cancel.is_cancelled() {
            return;
        }
        let Some(checks) = self.checks.clone() else { return };
        ui.note("[verify] running the project's tests");
        let acceptance = verify(checks, &self.dir, cancel).await;
        ui.note(&acceptance.summary());
        if let Some(provider) = report.provider {
            self.guards.scoreboard.record_acceptance(provider, &acceptance);
        }
        report.acceptance = Some(acceptance);
    }
}
