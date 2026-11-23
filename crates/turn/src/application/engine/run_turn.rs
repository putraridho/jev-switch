//! One turn from prompt to report: judge it, snapshot the tree, run the attempts.

use serde_json::Value;

use jev_session::domain::turn_status::TurnStatus;
use jev_changes::application::track_turn::{snapshot_until, summarize_until};
use jev_changes::domain::change_set::ChangeSummary;
use jev_changes::domain::snapshot::Snapshot;
use jev_changes::domain::turn_activity::TurnActivity;
use jev_judgment::domain::want::CodeWant;
use crate::application::cancel::{stop_signal, Cancel};
use crate::application::judged::Judged;
use crate::application::ports::Ui;
use crate::application::turn_report::TurnReport;
use super::Engine;

impl Engine {
    /// Run one user turn that cannot be cancelled.
    pub async fn run(&mut self, prompt: &str, ui: &mut dyn Ui) -> TurnReport {
        self.run_cancellable(prompt, ui, &Cancel::new()).await
    }

    /// Run one user turn that `cancel` can stop: the Jev call and the running CLI (with
    /// everything it started) are dropped at once, and the report says
    /// [`TurnStatus::Cancelled`].
    pub async fn run_cancellable(&mut self, prompt: &str, ui: &mut dyn Ui, cancel: &Cancel) -> TurnReport {
        let judged = tokio::select! {
            j = self.judge(prompt, ui) => j,
            _ = cancel.cancelled() => {
                self.images.clear();
                return self.cancelled_report(prompt, None, 0, ui);
            }
        };
        let report = self.run_judged_cancellable(prompt, judged, ui, cancel).await;
        // Attachments belong to the turn they were sent with.
        self.images.clear();
        report
    }

    /// Ask Jev. The only network call of a turn before the CLI starts.
    pub(super) async fn judge(&self, prompt: &str, ui: &mut dyn Ui) -> Judged {
        let recent_raw = self.sessions.recent_context(3, 300);
        let previous_raw = self.previous.as_ref().map(|(p, r)| format!("{p}\n{r}")).unwrap_or_default();
        // Jev is a cloud service: when the privacy policy lets this text go to no cloud
        // provider (local-only, a rule blocking it from all), Jev is not asked either.
        let everything = format!("{prompt}\n{recent_raw}\n{previous_raw}");
        if !self.guards.may_leave(&self.dir, &everything, &self.images) {
            ui.note("[jev] not asked: the privacy policy keeps this prompt on this machine");
            return if self.enabled {
                Judged::Providers(None)
            } else {
                let tier = self.sticky.tier.unwrap_or(1);
                Judged::Legacy(CodeWant { tier, confident: false, reason: "jev not asked (privacy policy)".into(), signals: Value::Null.into() })
            };
        }
        // What does go to Jev has its secrets blanked, as for any provider.
        let blank = |text: &str| crate::domain::privacy::outgoing::prepare(&self.guards.privacy, text).prompt;
        let (prompt, recent) = (blank(prompt), blank(&recent_raw));
        let previous_pair = self.previous.as_ref().map(|(p, r)| (blank(p), blank(r)));
        if !self.enabled {
            let want = self.router.code_want(&prompt, &recent, self.sticky.tier.unwrap_or(1)).await;
            if want.reason.starts_with("jev unavailable") {
                ui.note(&format!("[jev] {}; keeping current model", want.reason));
            }
            return Judged::Legacy(want);
        }
        let previous = previous_pair.as_ref().map(|(p, r)| (p.as_str(), r.as_str()));
        let want = self.router.provider_want(&prompt, &recent, previous, self.orchestrate).await;
        if want.is_none() {
            ui.note("[jev] unavailable (timeout or error); routing without its judgment");
        }
        Judged::Providers(want)
    }

    /// Run a turn whose Jev judgment is already in hand: choose, hand off, run, and
    /// fail over. Split from [`Engine::run`] so everything after the network call is
    /// testable with fake backends.
    pub async fn run_judged(&mut self, prompt: &str, judged: Judged, ui: &mut dyn Ui) -> TurnReport {
        self.run_judged_cancellable(prompt, judged, ui, &Cancel::new()).await
    }

    /// [`Engine::run_judged`] that `cancel` can stop. Around the turn proper it takes
    /// the working-tree snapshots (for [`TurnReport::changes`]) and saves the session.
    pub async fn run_judged_cancellable(&mut self, prompt: &str, judged: Judged, ui: &mut dyn Ui, cancel: &Cancel) -> TurnReport {
        let mut before: Option<Snapshot> = None;
        self.turn_activity = TurnActivity::default();
        self.quality.begin_turn();
        // A failed check of the turn before is told to this one, ahead of the request.
        let sent = self.with_check_context(prompt);
        let mut report = self.run_attempts(&sent, judged, ui, cancel, &mut before).await;
        if report.attempts == 0 {
            self.keep_check_context(&sent, prompt);
        } else {
            // A turn the user cancelled has spent its signal, so its summary is not raced
            // against it; a Ctrl-C while summarising a finished turn still returns at once.
            report.changes = self.changes_since(before, self.turn_activity.clone(), cancel, report.status != TurnStatus::Cancelled).await;
            let verify_ms = self.verify_report(&mut report, ui, cancel).await;
            self.grade(&mut report, verify_ms);
            self.last_changes = report.changes.clone();
            self.change_history.extend(report.changes.clone());
            self.save_quietly();
        }
        report
    }

    /// The working tree as a [`Snapshot`], or `None` (changes are not tracked, not a repo,
    /// git failed, or `cancel` fired while `honor_cancel`; git then finishes in the background).
    pub(super) async fn snapshot_tree(&self, cancel: &Cancel, honor_cancel: bool) -> Option<Snapshot> {
        if !self.track_changes {
            return None;
        }
        snapshot_until(self.working_tree.clone()?, self.dir.clone(), stop_signal(cancel, honor_cancel)).await
    }

    /// What differs from the `before` snapshot now; `None` if there is no `before`, git
    /// fails, or nothing changed.
    async fn changes_since(&self, before: Option<Snapshot>, activity: TurnActivity, cancel: &Cancel, honor_cancel: bool) -> Option<ChangeSummary> {
        let before = before?;
        let after = self.snapshot_tree(cancel, honor_cancel).await?;
        let summary = summarize_until(self.working_tree.clone()?, self.dir.clone(), before, after, activity, stop_signal(cancel, honor_cancel)).await?;
        (!summary.is_empty()).then_some(summary)
    }
}
