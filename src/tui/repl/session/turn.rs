//! Running one turn for the REPL: sending a message, keeping it in the session's log, and
//! what follows it (the summary, the bell, the changed files).

use std::time::{Duration, Instant};

use crate::tui::render as uifmt;
use crate::tui::repl::session::turn_race::{run_turn, Interrupt, TurnEnd};
use crate::tui::repl::session::{Flow, Io, Repl};
use crate::tui::views::status_facts as views;
use crate::turn::application::turn_report::TurnReport;
use crate::session::domain::session_log::TurnRecord;
use crate::session::domain::turn_status::TurnStatus;

impl Repl {
    /// A message from the person: sent with the notes waiting for it, attachments said.
    pub(crate) async fn prompt(&mut self, text: &str, io: &mut Io<'_>) -> Flow {
        if let Some(flow) = self.auto_compact_first(io).await {
            return flow;
        }
        let typed = self.take_attachments(text);
        let sent = self.with_carry(&typed);
        let before = self.engine.pin().clone();
        let routed = self.apply_team_route(&typed);
        let flow = self.run_recorded(&sent, text, io).await;
        if routed {
            self.engine.set_pin(before);
        }
        flow
    }

    /// Run `sent` as a turn and keep it in the session's log under `label`. The waiting
    /// notes are dropped once a model was started (they went with it).
    pub(crate) async fn run_recorded(&mut self, sent: &str, label: &str, io: &mut Io<'_>) -> Flow {
        // The marker names what the person asked, not the notes that went along with it.
        self.mark_turn_started(label);
        let started = Instant::now();
        let end = run_turn(&mut self.engine, &self.console, sent, io.interrupt).await;
        self.mark_turn_ended();
        match end {
            TurnEnd::Aborted => {
                self.audit_aborted(label, started.elapsed());
                Flow::Aborted
            }
            TurnEnd::Done(report, elapsed) => {
                // The notes (a `/compact` summary, `!command` output) are kept until a model
                // has answered with them: a failed or stopped turn must not lose them.
                if report.status == TurnStatus::Answered {
                    self.carry.clear();
                }
                self.record(label, &report, elapsed);
                self.finish(&report, elapsed);
                Flow::Continue
            }
        }
    }

    /// Add a finished turn to the session's log.
    fn record(&mut self, prompt: &str, report: &TurnReport, elapsed: Duration) {
        let route = report.provider.zip(report.model.clone());
        let model = route.as_ref().map(|(p, m)| format!("{p} · {m}")).unwrap_or_default();
        let record = TurnRecord {
            prompt: prompt.to_string(),
            answer: report.text.clone(),
            status: report.status,
            model,
            route,
            usage: report.usage,
            elapsed_ms: elapsed.as_millis() as u64,
            ended_ms: self.now(),
            tool_calls: report.tool_calls,
        };
        self.audit_turn(&record);
        self.log.push(record);
    }

    /// `/new`, `/clear`, `/resume`: nothing of the old conversation stays in this session.
    pub(crate) fn fresh_session(&mut self) {
        self.engine.new_session();
        self.last_summary = None;
        self.log.clear();
        self.carry.clear();
    }

    /// What follows a turn: the full "nothing can run" message when that was the outcome,
    /// else the summary line, the changed files and how to review them.
    pub(crate) fn finish(&mut self, report: &TurnReport, elapsed: Duration) {
        let style = self.console.err_style();
        if let Some(task) = self.console.take_no_route() {
            let rows = views::providers::avail_rows(&self.engine.runtime, self.now());
            self.console.err(&format!("{}\n", uifmt::no_route_message(&task, &rows, style)));
        }
        if report.attempts == 0 && report.status != TurnStatus::Cancelled {
            // Nothing was started, so there is no summary to give.
            self.console.err("");
            return;
        }
        let facts = views::turns::summary_facts(report, elapsed.as_millis() as u64, &self.engine.runtime);
        let files = report.changes.as_ref().map(views::turns::file_changes).unwrap_or_default();
        self.console.turn_end(&facts, &files);
        let outcome = if report.status == TurnStatus::Answered { "answered" } else { "failed" };
        crate::approval::application::extension_hooks::runner::fire(self.engine.hooks(), crate::approval::application::extension_hooks::Event::TurnEnd, None, Some(outcome));
        if bell_after(elapsed, crate::shared::infrastructure::env_file::lookup("JEV_BELL").as_deref()) {
            self.console.bell();
        }
        // Changes other processes made during the turn: said apart from the turn's own files.
        let warning = report.changes.as_ref().and_then(|c| c.outside.warning());
        if let Some(w) = warning {
            self.console.err(&format!("  {} {}\n", self.console.err_style().yellow("⚠"), w));
        }
        self.last_summary = Some(facts);
    }

    /// `-p`: one turn; the answer alone on stdout (laid out for the terminal when stdout is
    /// one), everything else on stderr. The process exit code.
    pub async fn one_shot(&mut self, prompt: &str, interrupt: &mut dyn Interrupt) -> i32 {
        let prompt = self.take_attachments(prompt);
        match run_turn(&mut self.engine, &self.console, &prompt, interrupt).await {
            TurnEnd::Aborted => 130,
            TurnEnd::Done(report, elapsed) => {
                if report.status == TurnStatus::Answered {
                    let text = uifmt::answer_text(report.text.trim_end(), self.out_style(), self.console.width());
                    self.console.out(&text);
                }
                self.finish(&report, elapsed);
                views::turns::exit_code(report.status)
            }
        }
    }
}

/// A turn at least this long rings the bell when it ends, so the person can look away.
const BELL_AFTER: Duration = Duration::from_secs(15);

/// Whether a turn that took `elapsed` rings the bell: long enough, and `JEV_BELL` is not off.
pub(super) fn bell_after(elapsed: Duration, setting: Option<&str>) -> bool {
    let off = setting.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"));
    elapsed >= BELL_AFTER && !off
}
