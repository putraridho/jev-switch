//! One worker attempt: pick and claim a model, make the worktree, run the CLI, read what it
//! changed and have its tests run.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use crate::approval::domain::mode::Mode;
use crate::backend::{self, Event, ProviderId, TurnError, TurnOutcome, TurnRequest};
use crate::catalog::domain::model_table::ModelSpec;
use crate::orchestration::domain::brief_signals::{worker_task, BriefSignals};
use crate::orchestration::domain::delegation::{DelegateRequest, DelegateResult, Landing, ReviewReason};
use crate::orchestration::domain::file_patch::{split_patch, truncate_with_note, FileDiff, FileStat, MAX_SUMMARY_CHARS};
use crate::orchestration::domain::worker_prompt::{plan_mode_enforced, worker_prompt};
use crate::orchestration::infrastructure::delegation_log::{append_attempt, AttemptLog};

use crate::orchestration::application::ports::CheckRun;
use super::outcome::{AttemptFailure, AttemptOutcome, Finished, Pending, Verdict};
use super::reservation::{create_unless_cancelled, CreateSlot, Reservation};
use super::{lock, Delegator};

impl Delegator {
    /// Run the worker CLI in its worktree. Plan usage it reports feeds the spreading.
    pub(super) async fn run_worker(&self, id: &str, spec: &'static ModelSpec, dir: &Path, prompt: String, mode: Mode) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
        let backend = (self.backend_factory)(spec.provider);
        let req = TurnRequest { dir: dir.to_path_buf(), model: spec.id.to_string(), prompt, images: vec![], session_id: None, mode, trust_project_settings: crate::approval::application::extension_hooks::trust::project_settings_trusted(dir) };
        let mut tools: Vec<String> = Vec::new();
        let mut on_event = |e: &Event| match e {
            Event::Tool(summary) => {
                tools.push(summary.clone());
                if let Some(records) = &self.records {
                    records.note_activity(id, tools.len(), summary);
                }
            }
            Event::Plan(plan) => lock(&self.runtime).note_plan(spec.provider, plan.clone()),
            _ => {}
        };
        let result = backend::run_turn(&*backend, &req, self.idle_timeout, &mut on_event).await;
        (result, tools)
    }

    pub(super) fn log_attempt(&self, a: &AttemptLog) {
        let Some(path) = &self.log_path else { return };
        append_attempt(path, &self.planner, &self.worktrees.repo(), a);
    }

    /// One attempt: pick and claim, create the worktree, run. `Err` is the failure to react
    /// to. If the future is dropped on the way (the planner cancelled the call) the
    /// [`Reservation`] removes the placeholder, its record and any worktree already made.
    pub(super) async fn attempt(
        &self,
        req: &DelegateRequest,
        signals: &BriefSignals,
        jev: &Value,
        excluded: &HashSet<(ProviderId, String)>,
        attempt: u32,
    ) -> Result<Finished, AttemptOutcome> {
        let id = self.next_id();
        let spec = self.reserve(signals, excluded, req.brief.len(), &id).map_err(|e| AttemptOutcome::Stop(e.to_string()))?;
        let reservation = Reservation::new(self, &id);
        let out = self.run_attempt(req, signals, jev, spec, id, &reservation, attempt).await;
        reservation.disarm();
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_attempt(
        &self,
        req: &DelegateRequest,
        signals: &BriefSignals,
        jev: &Value,
        spec: &'static ModelSpec,
        id: String,
        reservation: &Reservation<'_>,
        attempt: u32,
    ) -> Result<Finished, AttemptOutcome> {
        // A read-only worker on a CLI that enforces plan mode reads the user's own folder (and
        // so sees uncommitted edits); every other worker gets a worktree.
        let in_place = req.read_only && plan_mode_enforced(spec.provider);
        let path = if in_place {
            self.worktrees.repo()
        } else {
            let made = self.blocking({
                let (id, slot) = (id.clone(), reservation.slot());
                move |w| create_unless_cancelled(w, &id, &slot)
            });
            match made.await {
                Ok(path) => path,
                Err(e) => {
                    self.take(&id).await;
                    return Err(AttemptOutcome::Stop(format!("could not create a worktree: {e:#}")));
                }
            }
        };
        if !in_place {
            self.update(&id, |w| w.path = Some(path.clone())).await;
        }
        let started = Instant::now();
        let started_ms = (self.now)();
        // Secrets are blanked as for any provider, and the audit log notes what was sent.
        let prompt = self.guards.outgoing(&worker_prompt(&req.brief, &req.files, req.read_only), spec.provider, spec.id, (self.now)()).prompt;
        let mode = if req.read_only { Mode::Plan } else { Mode::default() };
        let (result, tools) = self.run_worker(&id, spec, &path, prompt, mode).await;
        self.update(&id, |w| w.running = false).await;
        let wall_ms = started.elapsed().as_millis() as u64;
        let task = worker_task(signals);
        let log = |result: &Result<TurnOutcome, TurnError>, files: &[String], landing: Option<&'static str>| {
            self.log_attempt(&AttemptLog { spec, id: &id, task, attempt, read_only: req.read_only, jev, result, wall_ms, started_ms, files, tools: &tools, landing, in_place })
        };
        let out = match result {
            Err(err) => {
                log(&Err(err.clone()), &[], None);
                // A failed worker's partial work is never offered; its worktree goes.
                if in_place {
                    self.take(&id).await;
                } else {
                    let _ = self.forget(&id).await;
                }
                return Err(AttemptOutcome::Failed(AttemptFailure { id, spec, err }));
            }
            Ok(out) => out,
        };
        let summary = truncate_with_note(out.text.trim(), MAX_SUMMARY_CHARS, "report");
        let done = |landing: Landing, stats: Vec<FileStat>, diff: String, shown: Vec<String>| DelegateResult {
            worker_id: id.clone(),
            provider: Some(spec.provider),
            model: Some(spec.id.to_string()),
            summary: summary.clone(),
            stats,
            diff,
            shown,
            ok: true,
            error: None,
            retries: vec![],
            landing,
        };
        if in_place {
            self.take(&id).await;
            let landing = Landing::Report { discarded: vec![] };
            log(&Ok(out), &[], Some(landing.tag()));
            return Ok(Finished::done(done(landing, vec![], String::new(), vec![])));
        }
        let read = self.blocking({
            let path = path.clone();
            move |w| w.diff(&path)
        });
        let diff = match read.await {
            Ok(diff) => diff,
            Err(e) => {
                log(&Ok(out), &[], None);
                let _ = self.forget(&id).await;
                return Err(AttemptOutcome::Stop(format!("worker {id} finished but its changes could not be read: {e:#}")));
            }
        };
        let parts = split_patch(&diff);
        let stats: Vec<FileStat> = parts.iter().map(FileDiff::stat).collect();
        let files: Vec<String> = stats.iter().map(|s| s.path.clone()).collect();
        // A read-only worker's worktree, and an editor's that changed nothing, only held a report.
        if req.read_only || parts.is_empty() {
            let _ = self.forget(&id).await;
            let landing = if req.read_only { Landing::Report { discarded: files.clone() } } else { Landing::NoChanges };
            log(&Ok(out), &files, Some(landing.tag()));
            return Ok(Finished::done(done(landing, vec![], String::new(), vec![])));
        }
        let verdict = self.verify(&path, req.review, reservation.slot()).await;
        log(&Ok(out), &files, Some(verdict.tag()));
        // The landing is decided by `settle`, when nobody is editing the main tree.
        Ok(Finished::Pending(Box::new(Pending { result: done(Landing::NoChanges, stats, String::new(), vec![]), patch: diff, verdict })))
    }

    /// Whether a finished editing worker's work may merge on its own: the project's tests,
    /// run in its worktree, must pass. Touches nothing outside the worktree.
    pub(super) async fn verify(&self, worktree: &Path, review: bool, slot: Arc<Mutex<CreateSlot>>) -> Verdict {
        if review {
            return Verdict::Review(ReviewReason::Asked);
        }
        if !self.auto_merge {
            return Verdict::Review(ReviewReason::Off);
        }
        let (checks, dir) = (self.checks.clone(), worktree.to_path_buf());
        // The run stops early if the delegation is cancelled: its worktree is going away.
        let ran = Self::off_thread(move || {
            let command = checks.test_command(&dir)?;
            let run = checks.run(&dir, &command, &|| lock(&slot).cancelled);
            Some((command, run))
        });
        let (command, run) = match ran.await {
            Ok(Some(found)) => found,
            Ok(None) => return Verdict::Review(ReviewReason::NoTestCommand),
            Err(e) => return Verdict::Review(ReviewReason::TestsNotRun { command: "the tests".into(), error: format!("{e:#}") }),
        };
        Verdict::Review(match run {
            CheckRun::Passed => return Verdict::Passed { command },
            CheckRun::Failed { output } => ReviewReason::TestsFailed { command, output },
            CheckRun::TimedOut { after } => ReviewReason::TestsTimedOut { command, after },
            CheckRun::Stopped => ReviewReason::TestsNotRun { command, error: "the delegation was cancelled".into() },
            CheckRun::CouldNotStart { error } => ReviewReason::TestsNotRun { command, error },
        })
    }
}
