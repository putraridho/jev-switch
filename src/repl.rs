//! The session: what each typed command does, and how one turn runs. The binary reads
//! lines (from the line editor or a pipe) and hands them to [`Repl::handle`]; everything
//! that follows is here, over an [`Engine`] and a [`Console`], so it is tested with fake
//! backends instead of a terminal.
//!
//! A turn races the engine against Ctrl-C ([`run_turn`]): the first Ctrl-C stops the turn
//! (the engine kills the model CLI and everything it started, and reports `Cancelled`), a
//! second one while it is stopping gives up on waiting.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::commands::Command;
use crate::console::Console;
use crate::doctor;
use crate::handoff::ResumeError;
use crate::log;
use crate::permissions::{self, PermissionsCmd};
use crate::picker;
use crate::routing::Pin as ModelPin;
use crate::terminal;
use crate::turn::{Cancel, Engine, TurnReport, TurnStatus};
use crate::typeahead::TypeAhead;
use crate::uifmt::{self, Style};
use crate::views;
use crate::changes::UndoError;
use crate::sessionlog::{SessionLog, TurnRecord};

mod diff_cmds;
mod cache_cmds;
mod library_cmds;
mod project_cmds;
mod ext_cmds;
mod team_cmds;
mod headless;
pub use headless::sleeper;
mod session_cmds;

// ------------------------------------------------------------------- interrupts

/// A source of Ctrl-C presses.
pub trait Interrupt: Send {
    /// Resolves at the next press.
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// Forget presses that arrived while no turn was running, so a stale one does not stop
    /// the next turn the moment it starts.
    fn drain(&mut self);
}

/// SIGINT as a stream, installed once for the whole run. Installing it early means a
/// Ctrl-C between two turns is queued instead of killing the process (the default action).
#[cfg(unix)]
pub struct SigInt(tokio::signal::unix::Signal);

#[cfg(unix)]
impl SigInt {
    pub fn install() -> std::io::Result<Self> {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map(Self)
    }
}

#[cfg(unix)]
impl Interrupt for SigInt {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // A closed stream never fires: it must not look like a Ctrl-C.
            if self.0.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        })
    }

    fn drain(&mut self) {
        use futures_util::FutureExt;
        while self.0.recv().now_or_never().flatten().is_some() {}
    }
}

#[cfg(not(unix))]
pub struct SigInt;

#[cfg(not(unix))]
impl SigInt {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(not(unix))]
impl Interrupt for SigInt {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            let _ = tokio::signal::ctrl_c().await;
        })
    }

    fn drain(&mut self) {}
}

/// Answers a yes/no question through the terminal.
pub trait Confirm: Send {
    fn confirm(&mut self, question: &str) -> bool;
}

/// How a turn ended, as far as the loop cares.
#[derive(Debug)]
pub enum TurnEnd {
    Done(Box<TurnReport>, Duration),
    /// A second Ctrl-C while the turn was stopping: the engine's future was dropped (which
    /// kills the model CLI and its children) and the program should exit.
    Aborted,
}

/// The next Ctrl-C that is meant for the turn. One pressed while an approval's pager is open
/// went to the pager too (it ends a search there) and does not stop the turn.
async fn press_outside_pager(interrupt: &mut dyn Interrupt, console: &Console) {
    loop {
        interrupt.wait().await;
        if !console.is_paging() {
            return;
        }
    }
}

/// Which key asked a turn to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopKey {
    CtrlC,
    Esc,
}

/// The next press that stops the turn: Ctrl-C (outside a pager), or Esc when keys are read
/// during the turn.
async fn stop_pressed(interrupt: &mut dyn Interrupt, console: &Console, ahead: Option<&TypeAhead>) -> StopKey {
    let esc = async {
        match ahead {
            Some(a) => a.escaped().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = press_outside_pager(interrupt, console) => StopKey::CtrlC,
        _ = esc => StopKey::Esc,
    }
}

/// Run one turn that Ctrl-C (or Esc) can stop. The status line shows while it runs and is
/// gone when this returns. With type-ahead on, keys typed meanwhile queue messages on the
/// console; a Ctrl-C stop drops that queue (Esc keeps it: it is how a message is sent now).
pub async fn run_turn(engine: &mut Engine, console: &Console, prompt: &str, interrupt: &mut dyn Interrupt) -> TurnEnd {
    interrupt.drain();
    let cancel = Cancel::new();
    let started = Instant::now();
    console.begin_turn();
    let _ticker = console.ticker();
    let ahead = console.type_ahead().then(|| TypeAhead::start(console.clone()));
    let mut ui = console.clone();
    let mut stopped_by: Option<StopKey> = None;
    let report = {
        let turn = engine.run_cancellable(prompt, &mut ui, &cancel);
        tokio::pin!(turn);
        tokio::select! {
            report = &mut turn => Some(report),
            key = stop_pressed(interrupt, console, ahead.as_ref()) => {
                stopped_by = Some(key);
                cancel.cancel();
                console.set_activity("stopping...");
                tokio::select! {
                    report = &mut turn => Some(report),
                    // Dropping `turn` at the end of this block kills what is still running.
                    _ = press_outside_pager(interrupt, console) => None,
                }
            }
        }
    };
    // A pager still open (its question was withdrawn) keeps the terminal until it is closed.
    console.pager_closed().await;
    if let Some(a) = ahead {
        a.finish();
    }
    // Ctrl-C stops a turn as a signal, or as a key at an approval question (which reads keys
    // raw): either way it is not Esc, and the queue goes.
    let cancelled = report.as_ref().is_some_and(|r| r.status == TurnStatus::Cancelled);
    if stopped_by == Some(StopKey::CtrlC) || (cancelled && stopped_by != Some(StopKey::Esc)) {
        let dropped = console.clear_queue();
        if dropped > 0 {
            let noun = if dropped == 1 { "message" } else { "messages" };
            console.err(&format!("  {}", console.err_style().dim(&format!("dropped {dropped} queued {noun} (Ctrl-C); Esc stops and keeps them"))));
        }
    }
    console.end_turn();
    match report {
        Some(r) => TurnEnd::Done(Box::new(r), started.elapsed()),
        None => TurnEnd::Aborted,
    }
}

// ------------------------------------------------------------------------ REPL

/// What the loop does after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
    /// Exit at once (a second Ctrl-C during a stopping turn).
    Aborted,
}

/// The terminal-facing parts one command may need.
pub struct Io<'a> {
    pub interrupt: &'a mut dyn Interrupt,
    /// Present only in an interactive session; without it `/undo` refuses.
    pub confirm: Option<&'a mut dyn Confirm>,
}

pub struct Repl {
    pub engine: Engine,
    pub console: Console,
    home: Option<String>,
    /// Whether stdout is a terminal, which decides if long output may go through a pager.
    paging: bool,
    last_summary: Option<uifmt::SummaryFacts>,
    /// The turns of this session, for `/cost`, `/export` and `/copy`.
    log: SessionLog,
    /// Notes that go with the next message: `!command` output, a `/compact` summary.
    carry: Vec<String>,
    /// Where scratch files go (`/edit`); the temp folder when `None`.
    state_dir: Option<std::path::PathBuf>,
    /// The last `/sessions` listing, which its numbers refer to.
    library: library_cmds::LibraryState,
}

impl Repl {
    pub fn new(engine: Engine, console: Console, home: Option<String>, paging: bool) -> Self {
        Self { engine, console, home, paging, last_summary: None, log: SessionLog::default(), carry: Vec::new(), state_dir: None, library: Default::default() }
    }

    /// Keep the mode the person just chose for the next session. Only their choices come
    /// here: the read-only mode `/review` and `/compact` borrow for one turn is not kept.
    pub fn remember_mode(&self) {
        if let Some(dir) = &self.state_dir {
            self.said_if_failed(crate::mode::save(dir, self.engine.mode()), "the mode");
        }
    }

    /// Keep an on/off setting the person just chose (`/orchestrate`, `/provider`).
    fn remember_flag(&self, flag: crate::prefs::Flag, on: bool, what: &str) {
        if let Some(dir) = &self.state_dir {
            self.said_if_failed(crate::prefs::save_flag(dir, flag, on), what);
        }
    }

    fn said_if_failed(&self, saved: std::io::Result<()>, what: &str) {
        if let Err(e) = saved {
            self.console.err(&format!("  could not keep {what} for the next session: {e}"));
        }
    }

    /// Keep scratch files (the message `/edit` composes) in the state folder.
    pub fn with_state_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.state_dir = dir;
        self
    }

    /// The session's turns so far.
    pub fn log(&self) -> &SessionLog {
        &self.log
    }

    fn now(&self) -> u64 {
        log::now_ms()
    }

    /// The worker records on disk (none without a state directory).
    fn stored_workers(&self) -> Vec<crate::delegate::StoredWorker> {
        log::state_dir().map(|d| crate::delegate::stored_workers(&d)).unwrap_or_default()
    }

    fn out_style(&self) -> Style {
        self.console.out_style()
    }

    /// Print command output, followed by a blank line.
    fn page_out(&self, text: &str) {
        self.console.out(text);
        self.console.out("");
    }

    /// Run one line's worth of input.
    pub async fn handle(&mut self, cmd: Command, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        match cmd {
            Command::Empty => {}
            Command::Quit => return Flow::Quit,
            Command::Prompt(text) => return self.prompt(&text, io).await,
            Command::Help => self.page_out(&uifmt::help_text(style, self.console.width())),
            Command::Learn(learn) => self.page_out(&learn.render(style, self.console.width())),
            Command::Status => {
                let facts = views::status_facts(&self.engine, self.last_summary.as_ref(), self.home.as_deref(), self.now());
                self.page_out(&uifmt::status_text(&facts, style));
            }
            Command::Providers { refresh } => self.providers(refresh).await,
            Command::Mcp(cmd) => self.mcp(cmd),
            Command::Doctor => {
                self.console.out(&style.dim("  checking the setup..."));
                let text = self.doctor_text().await;
                self.page_out(&text);
            }
            Command::ShowModel => self.console.out(&format!("  model: {}", self.engine.pin())),
            Command::SetPin(pin) => self.set_pin(pin),
            Command::SetRouting(on) => {
                self.engine.set_enabled(on);
                self.remember_flag(crate::prefs::Flag::Routing, on, "routing");
                let text = if on { "on".to_string() } else { format!("off {}", style.dim("(Claude only)")) };
                self.console.out(&format!("  routing: {text}"));
            }
            Command::SetOrchestrate(on) => {
                self.engine.set_orchestrate(on);
                self.remember_flag(crate::prefs::Flag::Orchestrate, on, "orchestrate");
                self.console.out(&format!("  orchestrate: {}", on_off(on)));
            }
            Command::ShowOrchestrate => self.console.out(&format!("  orchestrate: {}", on_off(self.engine.orchestrate()))),
            Command::Trust(cmd) => return self.trust(cmd, io).await,
            Command::SetMode(mode) => {
                let mode = self.adopt_mode(mode);
                self.console.out(&format!("  mode: {}", views::mode_line(mode)));
            }
            Command::ShowMode => self.console.out(&format!("  mode: {}", views::mode_line(self.engine.mode()))),
            Command::New => {
                self.fresh_session();
                self.console.out(&format!("{} new session", style.green("✓")));
            }
            Command::Clear => {
                self.console.clear_screen();
                self.fresh_session();
                self.console.out(&format!("{} new session", style.green("✓")));
            }
            Command::Compact(focus) => return self.compact(focus.as_deref(), io).await,
            Command::Context => self.page_out(&self.context_text()),
            Command::Cost => self.page_out(&self.cost_text()),
            Command::Cache => self.page_out(&self.cache_text()),
            Command::Effort(setting) => self.effort(setting),
            Command::Guard(cmd) => {
                let text = self.engine.run_guard_command(cmd);
                self.page_out(&text);
            }
            Command::Why => {
                let pin = self.engine.pin().to_string();
                self.page_out(&uifmt::why_text(self.console.last_route().as_ref(), &pin, self.engine.enabled(), style));
            }
            Command::Export(path) => self.export(path.as_deref()),
            Command::Copy => self.copy(),
            Command::Edit => return self.edit(io).await,
            Command::Init => return self.canned(session_cmds::INIT_PROMPT, "/init", None, io).await,
            Command::Memory => self.memory(),
            Command::Review(focus) => {
                let prompt = session_cmds::review_prompt(focus.as_deref());
                return self.canned(&prompt, "/review", Some(crate::mode::Mode::Plan), io).await;
            }
            Command::Config => self.page_out(&self.config_text()),
            Command::Agents => self.page_out(&crate::agentview::stored::agents_text(&self.stored_workers(), self.now(), self.console.width(), style)),
            Command::Orchestration => {
                let text = crate::agentview::stored::orchestration_text(self.engine.orchestrate(), &self.stored_workers(), self.now(), self.console.width(), style);
                self.page_out(&text);
            }
            Command::TerminalSetup { install } => self.terminal_setup(install, io),
            Command::Update => self.update(io),
            Command::Shell(command) => self.shell(&command),
            Command::Note(note) => self.note(&note),
            Command::Resume(query) => self.resume(query.as_deref()),
            Command::Rename(name) => self.rename(name.as_deref()),
            Command::Workflows => {
                let text = self.workflows_text();
                self.page_out(&text);
            }
            Command::Sessions(cmd) => self.sessions(cmd),
            Command::Project(cmd) => return self.project(cmd, io).await,
            Command::Permissions(cmd) => self.permissions(cmd),
            Command::Diff(args) => self.diff(args.as_deref(), io),
            Command::Rewind(turn) => self.rewind(turn, io),
            Command::Undo => self.undo(io).await,
            Command::Invalid(message) => self.console.out(&format!("{} {message}", style.yellow("⚠"))),
            Command::Ext(cmd) => return self.ext_command(cmd, io).await,
            Command::Unknown { typed, args, suggestion } => {
                if let Some(flow) = self.custom(&typed, &args, io).await {
                    return flow;
                }
                let mut text = format!("{} unknown command {typed}", style.red("✗"));
                if let Some(s) = suggestion {
                    text.push_str(&format!(". Did you mean {}?", style.cyan(s)));
                }
                text.push_str(&format!(" Type {} for the commands.", style.cyan("/help")));
                self.console.out(&text);
            }
        }
        Flow::Continue
    }

    fn set_pin(&mut self, pin: ModelPin) {
        self.console.out(&format!("  model: {pin}"));
        self.engine.set_pin(pin);
    }

    // ----------------------------------------------------------------- turns

    /// A message from the person: sent with the notes waiting for it, attachments said.
    async fn prompt(&mut self, text: &str, io: &mut Io<'_>) -> Flow {
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
    async fn run_recorded(&mut self, sent: &str, label: &str, io: &mut Io<'_>) -> Flow {
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
        };
        self.audit_turn(&record);
        self.log.push(record);
    }

    /// `/new`, `/clear`, `/resume`: nothing of the old conversation stays in this session.
    fn fresh_session(&mut self) {
        self.engine.new_session();
        self.last_summary = None;
        self.log.clear();
        self.carry.clear();
    }

    /// What follows a turn: the full "nothing can run" message when that was the outcome,
    /// else the summary line, the changed files and how to review them.
    fn finish(&mut self, report: &TurnReport, elapsed: Duration) {
        let style = self.console.err_style();
        if let Some(task) = self.console.take_no_route() {
            let rows = views::avail_rows(&self.engine.runtime, self.now());
            self.console.err(&format!("{}\n", uifmt::no_route_message(&task, &rows, style)));
        }
        if report.attempts == 0 && report.status != TurnStatus::Cancelled {
            // Nothing was started, so there is no summary to give.
            self.console.err("");
            return;
        }
        let facts = views::summary_facts(report, elapsed.as_millis() as u64, &self.engine.runtime);
        let files = report.changes.as_ref().map(views::file_changes).unwrap_or_default();
        self.console.turn_end(&facts, &files);
        let outcome = if report.status == TurnStatus::Answered { "answered" } else { "failed" };
        crate::ext::hooks::fire(self.engine.hooks(), crate::ext::hooks::Event::TurnEnd, None, Some(outcome));
        if bell_after(elapsed, crate::envfile::lookup("JEV_BELL").as_deref()) {
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
                views::exit_code(report.status)
            }
        }
    }

    // -------------------------------------------------------------- commands

    async fn providers(&mut self, refresh: bool) {
        let style = self.out_style();
        if refresh {
            self.console.out(&style.dim("  checking providers..."));
            self.engine.refresh_providers().await;
        }
        let rows = views::avail_rows(&self.engine.runtime, self.now());
        let mut text = uifmt::providers_table(&rows, style, self.console.width());
        for note in &self.engine.runtime.overrides.notes {
            text.push_str(&format!("\n  {}", style.dim(&format!("override: {note}"))));
        }
        self.page_out(&text);
    }

    /// The doctor's screen for this session.
    pub async fn doctor_text(&self) -> String {
        let inputs = doctor::gather(&self.engine.runtime, self.engine.dir()).await;
        views::doctor_text(&doctor::diagnose(&inputs), self.out_style(), self.console.width())
    }

    /// `/permissions`: the rules of this project, and changing them. Claude Code reads the
    /// file itself, so a change counts from the next tool call.
    fn permissions(&mut self, cmd: PermissionsCmd) {
        let style = self.out_style();
        let dir = self.engine.dir().to_path_buf();
        let home = self.home.as_deref().map(std::path::Path::new);
        let saved = |what: &str| format!("{} {what} ({})", style.green("✓"), permissions::LOCAL_SETTINGS);
        let outcome: Result<Option<String>, String> = match cmd {
            PermissionsCmd::List => Ok(None),
            PermissionsCmd::Allow(rule) => permissions::add_allow_rule(&dir, &rule)
                .map(|added| Some(if added { saved(&format!("allowed {rule}")) } else { format!("  {}", style.dim(&format!("{rule} is already allowed"))) }))
                .map_err(|e| e.to_string()),
            PermissionsCmd::Remove(rule) => permissions::remove_allow_rule(&dir, &rule)
                .map(|removed| Some(if removed { saved(&format!("removed {rule}")) } else { format!("  {}", style.dim(&format!("{rule} is not in the project's rules (see /permissions)"))) }))
                .map_err(|e| e.to_string()),
            PermissionsCmd::Dir(arg) => permissions::resolve_dir(&arg, &dir, home).and_then(|path| {
                permissions::add_directory(&dir, &path)
                    .map(|added| Some(if added { saved(&format!("added folder {}", path.display())) } else { format!("  {}", style.dim(&format!("{} is already there", path.display()))) }))
                    .map_err(|e| e.to_string())
            }),
        };
        match outcome {
            Ok(Some(line)) => self.console.out(&line),
            Ok(None) => self.page_out(&uifmt::permissions_text(&permissions::listing(&dir, home), style, &dir)),
            Err(message) => self.console.out(&format!("{} {message}", style.yellow("⚠"))),
        }
    }

    /// `/resume [number | words]`: continue one of this folder's saved sessions. With
    /// several to choose from, a terminal gets the list to pick from; any other output gets
    /// it numbered, and `/resume 2` takes one.
    /// `/workflows`: orchestration in this project, from the worker registry and the log.
    fn workflows_text(&self) -> String {
        let now = self.now();
        let dir = self.engine.dir().to_path_buf();
        let open = self
            .state_dir
            .as_deref()
            .map(|s| crate::delegate::open_workers(s, &dir))
            .unwrap_or_default()
            .into_iter()
            .map(|w| uifmt::OpenWorkerRow { id: w.worker_id, provider: w.provider, model: w.model, age_ms: now.saturating_sub(w.created_ms), running: w.running, shown: w.shown, merged: w.merged })
            .collect();
        let log = log::path_from_env("JEV_DELEGATE_LOG", "orchestration.jsonl");
        let rows = log.map(|p| crate::delegate::recent_delegations(std::path::Path::new(&p), &dir, 200)).unwrap_or_default();
        let day_ago = now.saturating_sub(24 * 60 * 60 * 1000);
        let today: Vec<_> = rows.iter().filter(|d| d.ts_ms >= day_ago).collect();
        let day = (today.len(), today.iter().filter(|d| d.ok).count(), today.iter().map(|d| d.wall_ms).sum());
        let skip = rows.len().saturating_sub(10);
        let recent = rows
            .into_iter()
            .skip(skip)
            .map(|d| uifmt::DelegationRow {
                ago: views::human_ago(now.saturating_sub(d.ts_ms)),
                id: d.worker_id.unwrap_or_else(|| "-".into()),
                task: d.task.unwrap_or_else(|| "-".into()),
                provider: d.provider,
                model: d.model,
                ok: d.ok,
                error_kind: d.error_kind,
                wall_ms: d.wall_ms,
                files: d.files_changed.len(),
                tools: d.tool_calls.len(),
                attempt: d.attempt.unwrap_or(1),
            })
            .collect();
        let facts = uifmt::WorkflowFacts { orchestrate: self.engine.orchestrate(), open, recent, day };
        uifmt::workflows_text(&facts, self.out_style(), self.console.width())
    }

    /// `/rename [name]`: name the session, or say what it is called.
    fn rename(&mut self, name: Option<&str>) {
        let style = self.out_style();
        let Some(name) = name else {
            let text = match self.engine.sessions.name() {
                Some(n) => format!("This session is called \u{201c}{n}\u{201d}. Rename it with /rename <name>."),
                None => "This session has no name yet. Name it with /rename <name>; /resume then lists it by that name.".to_string(),
            };
            return self.console.out(&format!("  {}", style.dim(&text)));
        };
        self.engine.sessions.set_name(Some(name));
        let Some(set) = self.engine.sessions.name().map(String::from) else {
            return self.console.out(&format!("  {}", style.dim("A name needs at least one word.")));
        };
        let saved = self.engine.sessions.turns() > 0;
        let saving = self.engine.save_now();
        self.said_if_failed(saving, "the name");
        let when = if saved { "" } else { " (kept with your first message)" };
        self.console.out(&format!("{} this session is now called \u{201c}{set}\u{201d}{}", style.green("✓"), style.dim(when)));
    }

    fn resume(&mut self, query: Option<&str>) {
        let style = self.out_style();
        let all = match self.engine.resumable() {
            Ok(all) => all,
            Err(e) => return self.resume_failed(&e),
        };
        if all.is_empty() {
            return self.resume_failed(&ResumeError::NotFound);
        }
        let now = self.now();
        let candidates = match query.map(|q| (q, q.parse::<usize>())) {
            Some((_, Ok(n))) => match n.checked_sub(1).and_then(|i| all.get(i)) {
                Some(one) => vec![one.clone()],
                None => {
                    let text = format!("There is no session {n}; there are {}.", all.len());
                    return self.console.out(&format!("  {}", style.dim(&text)));
                }
            },
            Some((words, Err(_))) => {
                let found: Vec<_> = all.iter().filter(|e| picker::matches(e, words)).cloned().collect();
                if found.is_empty() {
                    return self.console.out(&format!("  {}", style.dim(&format!("No saved session matches \"{words}\"."))));
                }
                found
            }
            None => all,
        };
        let chosen = if candidates.len() == 1 {
            Some(0)
        } else if self.paging && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            self.console.with_terminal(|| picker::run(candidates.clone(), now, style, terminal::rows(), terminal::width()))
        } else {
            let mut lines = picker::numbered(&candidates, now, terminal::width(), style);
            lines.push(style.dim("  Resume one with /resume <number>, or narrow the list with /resume <words>."));
            self.console.out(&lines.join("\n"));
            return;
        };
        let Some(entry) = chosen.and_then(|i| candidates.get(i)) else {
            return self.console.out(&format!("  {}", style.dim("Resume cancelled.")));
        };
        match self.engine.resume_from(&entry.path) {
            Ok(info) => {
                self.last_summary = None;
                self.log.clear();
                self.carry.clear();
                self.console.out(&views::resumed_line(&info, self.now(), style));
            }
            Err(e) => self.resume_failed(&e),
        }
    }

    fn resume_failed(&self, e: &ResumeError) {
        let style = self.out_style();
        match e {
            ResumeError::NotFound | ResumeError::Disabled => {
                self.console.out(&format!("  {}", style.dim(&format!("Nothing to resume: {e}"))));
            }
            _ => {
                let steps = ["Start fresh with `/new`".to_string()];
                self.console.out(&uifmt::problem_block("The saved session can't be read", &[e.to_string()], &steps, style));
            }
        }
    }

    async fn undo(&mut self, io: &mut Io<'_>) {
        let style = self.out_style();
        let Some(summary) = self.engine.last_changes().cloned() else {
            self.console.out(&format!("  {}", style.dim("Nothing to undo: the last turn changed no files.")));
            return;
        };
        if let Some(m) = &summary.outside.moved {
            // The repository moved during the turn: refuse before listing anything to revert.
            let e = UndoError::RepoMoved(format!("the repository moved during the turn ({})", m.describe()));
            self.console.out(&uifmt::problem_block("Not undone", &[e.to_string()], &[], style));
            return;
        }
        if summary.files.is_empty() {
            self.console.out(&format!("  {}", style.dim("Nothing to undo: the last turn's own tools changed no files.")));
            return;
        }
        if let Some(w) = summary.outside.warning() {
            self.console.out(&format!("  {} {}", style.yellow("⚠"), w));
        }
        let files = views::file_changes(&summary);
        let noun = if files.len() == 1 { "file" } else { "files" };
        self.console.out(&format!("  Would revert {} {noun}:", files.len()));
        self.console.out(&uifmt::changes_block(&files, style, self.console.width()));
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!(
                "{} Not undone: asking needs a terminal. Run /undo in an interactive session.",
                style.yellow("⚠")
            ));
            return;
        };
        if !confirm.confirm(&format!("Revert {} {noun}? [y/N] ", files.len())) {
            self.console.out(&format!("  {}", style.dim("Left as it is.")));
            return;
        }
        match self.engine.undo_last() {
            Ok(r) => {
                let mut parts: Vec<String> = Vec::new();
                for (verb, list) in [("restored", &r.restored), ("removed", &r.removed), ("recreated", &r.recreated)] {
                    if !list.is_empty() {
                        parts.push(format!("{verb} {}", list.join(", ")));
                    }
                }
                self.console.out(&format!("{} reverted: {}", style.green("✓"), parts.join("; ")));
            }
            Err(UndoError::ChangedSince(changed)) => {
                let steps = [
                    "Look at what differs: `/diff`".to_string(),
                    "Revert by hand (for example `git checkout -- <file>`), or commit your edits first".to_string(),
                ];
                self.console.out(&uifmt::problem_block("Not undone: those files changed since the turn", &changed, &steps, style));
            }
            Err(e) => self.console.out(&uifmt::problem_block("Not undone", &[e.to_string()], &[], style)),
        }
    }
}

/// A turn at least this long rings the bell when it ends, so the person can look away.
pub const BELL_AFTER: Duration = Duration::from_secs(15);

/// Whether a turn that took `elapsed` rings the bell: long enough, and `JEV_BELL` is not off.
pub fn bell_after(elapsed: Duration, setting: Option<&str>) -> bool {
    let off = setting.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"));
    elapsed >= BELL_AFTER && !off
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// Show `text` through `$PAGER` (default `less -R`). `false` when no pager could run, so
/// the caller prints it itself.
pub fn page(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command as Process, Stdio};
    let (program, args) = views::pager_command(crate::envfile::lookup("PAGER").as_deref());
    let Ok(mut child) = Process::new(program).args(args).stdin(Stdio::piped()).spawn() else { return false };
    if let Some(mut stdin) = child.stdin.take() {
        // The pager may quit before reading everything; that is not an error.
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::{Availability, State};
    use crate::backend::{classify_common, Backend, ErrorKind, Event, Finish, ProviderId, TurnRequest, Usage};
    use crate::console::testing::Buf;
    use crate::console::ConsoleConfig;
    use crate::providers::Runtime;
    use crate::router::Router;
    use crate::turn::Judged;
    use crate::typesafe::TypeSafe;
    use std::sync::{Arc, Mutex};
    use tokio::process::Command as TokioCommand;

    mod project_tests;

    /// A model CLI that is a shell script printing canned lines: `TOOL:x`, `TEXT:x`, `FIN:x`.
    struct Fake {
        provider: ProviderId,
        script: String,
    }

    impl Backend for Fake {
        fn provider(&self) -> ProviderId {
            self.provider
        }
        fn command(&self, req: &TurnRequest) -> TokioCommand {
            let mut c = TokioCommand::new("sh");
            c.args(["-c", &self.script]).current_dir(&req.dir).env("FAKE_PROMPT", &req.prompt);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            match line.trim_end().split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish {
                    text: t.into(),
                    session_id: Some("s1".into()),
                    usage: Usage { input_tokens: 2000, cached_input_tokens: 1000, output_tokens: 50, context_tokens: Some(2000) },
                })],
                Some(("TOOL", t)) => vec![Event::Tool(t.into())],
                Some(("TEXT", t)) => vec![Event::Text(t.into())],
                Some(("FAIL", m)) => vec![Event::Failed { message: m.into() }],
                _ => vec![],
            }
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            classify_common(message).unwrap_or(ErrorKind::Other)
        }
    }

    /// An interrupt that never fires, or fires after a delay, once or twice.
    struct Presses {
        pending: Vec<Duration>,
    }

    impl Interrupt for Presses {
        fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            let delay = if self.pending.is_empty() { None } else { Some(self.pending.remove(0)) };
            Box::pin(async move {
                match delay {
                    Some(d) => tokio::time::sleep(d).await,
                    None => std::future::pending::<()>().await,
                }
            })
        }
        fn drain(&mut self) {}
    }

    struct Answer(bool, Arc<Mutex<Vec<String>>>);

    impl Confirm for Answer {
        fn confirm(&mut self, question: &str) -> bool {
            self.1.lock().unwrap().push(question.to_string());
            self.0
        }
    }

    pub(super) struct Rig {
        pub(super) repl: Repl,
        pub(super) out: Buf,
        pub(super) err: Buf,
        pub(super) dir: std::path::PathBuf,
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    pub(super) fn rig(name: &str, script: &str) -> Rig {
        let dir = std::env::temp_dir().join(format!("jev-repl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);
        let avail = vec![Availability { provider: ProviderId::Claude, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true }];
        let script = script.to_string();
        let engine = Engine::new(Runtime::for_tests(avail), Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), dir.clone())
            .with_turn_log(None)
            .with_backend_factory(Box::new(move |p| Box::new(Fake { provider: p, script: script.clone() })));
        let (out, err) = (Buf::default(), Buf::default());
        let cfg = ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: dir.clone(), stream_answers: true };
        let console = Console::new(Box::new(out.clone()), Box::new(err.clone()), cfg);
        Rig { repl: Repl::new(engine, console, None, false), out, err, dir }
    }

    pub(super) async fn run(r: &mut Rig, line: &str) -> Flow {
        let mut int = Presses { pending: vec![] };
        let mut io = Io { interrupt: &mut int, confirm: None };
        r.repl.handle(crate::commands::parse(line), &mut io).await
    }

    pub(super) const ANSWER: &str = "printf 'TOOL:Edit a.txt\\nTEXT:working\\nFIN:the answer\\n'; echo two >> a.txt";

    #[tokio::test]
    async fn help_status_and_unknown_commands_print_to_stdout() {
        let mut r = rig("info", ANSWER);
        assert_eq!(run(&mut r, "/help").await, Flow::Continue);
        assert!(r.out.text().contains("/model") && r.out.text().contains("Ctrl-C"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/status").await;
        let s = r.out.text();
        assert!(s.contains("Routing") && s.contains("on") && s.contains("claude"), "{s}");
        r.out.clear();
        run(&mut r, "/provders").await;
        let u = r.out.text();
        assert!(u.contains("unknown command /provders") && u.contains("Did you mean /providers?") && u.contains("/help"), "{u}");
        assert_eq!(run(&mut r, "/xyzzy").await, Flow::Continue);
        assert!(!r.out.text().contains("Did you mean /xyzzy"));
        assert_eq!(run(&mut r, "/quit").await, Flow::Quit);
        assert_eq!(r.err.text(), "", "no model was asked and nothing went to stderr");
    }

    #[tokio::test]
    async fn settings_commands_change_the_engine_and_say_so() {
        let mut r = rig("settings", ANSWER);
        run(&mut r, "/model codex").await;
        assert_eq!(r.repl.engine.pin().to_string(), "codex");
        run(&mut r, "/provider off").await;
        assert!(!r.repl.engine.enabled());
        run(&mut r, "/orchestrate on").await;
        assert!(r.repl.engine.orchestrate());
        run(&mut r, "/model").await;
        run(&mut r, "/model nonsense").await;
        let o = r.out.text();
        assert!(o.contains("model: codex") && o.contains("routing: off (Claude only)") && o.contains("orchestrate: on"), "{o}");
        assert!(o.contains("⚠ unknown provider"), "{o}");
        run(&mut r, "/providers").await;
        assert!(r.out.text().contains("● claude"), "{}", r.out.text());
    }

    async fn judged_turn(r: &mut Rig, prompt: &str) -> TurnEnd {
        let want = crate::router::ProviderWant {
            task: crate::catalog::TaskType::Feature,
            task_confidence: 0.9,
            very_hard_p: 0.0,
            security_p: 0.0,
            edit_p: 0.5,
            continues_p: 0.5,
            correction_p: None,
            orchestration: crate::router::Orchestration::default(),
            security_keywords: crate::router::Security::None,
            signals: serde_json::json!({}),
        };
        let started = Instant::now();
        let mut ui = r.repl.console.clone();
        r.repl.console.begin_turn();
        let report = r.repl.engine.run_judged(prompt, Judged::Providers(Some(want)), &mut ui).await;
        r.repl.console.end_turn();
        TurnEnd::Done(Box::new(report), started.elapsed())
    }

    #[tokio::test]
    async fn a_turn_prints_route_tools_answer_summary_and_the_review_hint() {
        let mut r = rig("turn", ANSWER);
        let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
        r.repl.finish(&report, elapsed);
        let (o, e) = (r.out.text(), r.err.text());
        assert!(o.contains("working") && o.contains("the answer"), "{o}");
        assert!(e.contains("→ claude") && e.contains("⏺ Edit a.txt"), "{e}");
        assert!(e.contains("✓ claude") && e.contains("1 file") && e.contains("/diff to review, /undo to revert"), "{e}");
        assert!(!e.contains("Answered") && !e.contains("TurnReport"), "no Debug output: {e}");
    }

    #[tokio::test]
    async fn diff_shows_the_turn_and_undo_asks_first_and_refuses_without_a_terminal() {
        let mut r = rig("undo", ANSWER);
        let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
        r.repl.finish(&report, elapsed);
        r.out.clear();

        run(&mut r, "/diff").await;
        let d = r.out.text();
        assert!(d.contains("+ two") && d.contains("a.txt"), "{d}");
        r.out.clear();
        run(&mut r, "/diff nothing.txt").await;
        assert!(r.out.text().contains("No changes to show for nothing.txt") || r.out.text().contains("No differences"), "{}", r.out.text());

        // No terminal: it lists what it would do and refuses.
        r.out.clear();
        run(&mut r, "/undo").await;
        let u = r.out.text();
        assert!(u.contains("Would revert 1 file") && u.contains("a.txt") && u.contains("Not undone: asking needs a terminal"), "{u}");
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n", "nothing was touched");

        // A "no" leaves it, a "yes" puts the file back.
        let asked = Arc::new(Mutex::new(Vec::new()));
        for (answer, expect_reverted) in [(false, false), (true, true)] {
            r.out.clear();
            let mut int = Presses { pending: vec![] };
            let mut yes = Answer(answer, asked.clone());
            let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
            r.repl.handle(Command::Undo, &mut io).await;
            let content = std::fs::read_to_string(r.dir.join("a.txt")).unwrap();
            assert_eq!(content == "one\n", expect_reverted, "{content:?} / {}", r.out.text());
        }
        assert_eq!(asked.lock().unwrap().as_slice(), ["Revert 1 file? [y/N] ", "Revert 1 file? [y/N] "]);
        assert!(r.out.text().contains("✓ reverted: restored a.txt"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/undo").await;
        assert!(r.out.text().contains("Nothing to undo"), "{}", r.out.text());
    }

    async fn two_edit_turns(name: &str) -> Rig {
        let mut r = rig(name, ANSWER);
        for prompt in ["change a", "change a again"] {
            let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, prompt).await else { panic!() };
            r.repl.finish(&report, elapsed);
        }
        r.out.clear();
        r
    }

    async fn run_with(r: &mut Rig, line: &str, answer: bool) -> Vec<String> {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut int = Presses { pending: vec![] };
        let mut yes = Answer(answer, asked.clone());
        let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
        r.repl.handle(crate::commands::parse(line), &mut io).await;
        let questions = asked.lock().unwrap().clone();
        questions
    }

    #[tokio::test]
    async fn diff_looks_at_one_turn_or_all_of_them_with_line_numbers() {
        let mut r = two_edit_turns("diff-scopes").await;
        run(&mut r, "/diff --list").await;
        let l = r.out.text();
        assert!(l.contains(" 1 a.txt") && l.contains(" 2 a.txt"), "{l}");

        r.out.clear();
        run(&mut r, "/diff --all").await;
        let all = r.out.text();
        assert!(all.contains("▸ a.txt  +2 −0") && all.contains("+ two"), "{all}");

        r.out.clear();
        run(&mut r, "/diff --turn 1").await;
        let one = r.out.text();
        assert!(one.contains("▸ a.txt  +1 −0") && one.contains("  1 1   one") && one.contains("2 + two"), "{one}");

        r.out.clear();
        run(&mut r, "/diff --turn 9").await;
        assert!(r.out.text().contains("there is no turn 9"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/diff --bogus").await;
        assert!(r.out.text().contains("unknown option --bogus"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn diff_exports_a_patch_that_git_applies() {
        let mut r = two_edit_turns("diff-export").await;
        run(&mut r, "/diff --all --export all.patch").await;
        assert!(r.out.text().contains("patch written to"), "{}", r.out.text());
        let ok = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&r.dir).args(args).status().unwrap().success();
        assert!(ok(&["checkout", "--", "a.txt"]));
        assert!(ok(&["apply", "all.patch"]));
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");
        r.out.clear();
        run(&mut r, "/diff --all --export all.patch").await;
        assert!(r.out.text().contains("already exists"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn review_reverts_only_the_hunks_the_user_rejects() {
        let mut r = two_edit_turns("diff-review").await;
        run(&mut r, "/diff --review").await;
        assert!(r.out.text().contains("needs a terminal"), "{}", r.out.text());

        r.out.clear();
        let asked = run_with(&mut r, "/diff --review", false).await;
        assert_eq!(asked, ["Revert this hunk? [y/N] "]);
        assert!(r.out.text().contains("hunk 1 of 1") && r.out.text().contains("1 kept, 0 reverted"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

        r.out.clear();
        run_with(&mut r, "/diff --review", true).await;
        assert!(r.out.text().contains("0 kept, 1 reverted") && !r.out.text().contains("/undo will refuse"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n");
    }

    #[tokio::test]
    async fn undo_and_rewind_still_work_after_a_hunk_was_reverted_in_review() {
        let mut r = two_edit_turns("review-then-undo").await;
        run_with(&mut r, "/diff --review", true).await;
        r.out.clear();
        run_with(&mut r, "/undo", true).await;
        assert!(r.out.text().contains("reverted:") && !r.out.text().contains("Not undone"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n");

        let mut r = two_edit_turns("review-then-rewind").await;
        run_with(&mut r, "/diff --review", true).await;
        r.out.clear();
        run_with(&mut r, "/rewind 1", true).await;
        assert!(r.out.text().contains("rewound: 2 turn(s) undone"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\n");
    }

    #[tokio::test]
    async fn rewind_lists_turns_and_undoes_from_the_chosen_one() {
        let mut r = two_edit_turns("rewind").await;
        run(&mut r, "/rewind").await;
        assert!(r.out.text().contains(" 1 a.txt") && r.out.text().contains("/rewind <number>"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/rewind 5").await;
        assert!(r.out.text().contains("There is no turn 5"), "{}", r.out.text());

        r.out.clear();
        run(&mut r, "/rewind 1").await;
        assert!(r.out.text().contains("Would undo 2 turns") && r.out.text().contains("needs a terminal"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

        assert_eq!(run_with(&mut r, "/rewind 1", false).await, ["Rewind to before turn 1? [y/N] "]);
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\ntwo\n");

        r.out.clear();
        run_with(&mut r, "/rewind 2", true).await;
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\ntwo\n", "only the second turn is undone");
        r.out.clear();
        run_with(&mut r, "/rewind 1", true).await;
        assert!(r.out.text().contains("rewound: 1 turn(s) undone"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\n");
        r.out.clear();
        run(&mut r, "/diff --all").await;
        assert!(r.out.text().contains("not changed any files"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn undo_that_would_lose_later_edits_explains_instead_of_failing() {
        let mut r = rig("undo-late", ANSWER);
        let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
        r.repl.finish(&report, elapsed);
        std::fs::write(r.dir.join("a.txt"), "my own edit\n").unwrap();
        r.out.clear();
        let mut int = Presses { pending: vec![] };
        let mut yes = Answer(true, Arc::new(Mutex::new(Vec::new())));
        let mut io = Io { interrupt: &mut int, confirm: Some(&mut yes) };
        r.repl.handle(Command::Undo, &mut io).await;
        let o = r.out.text();
        assert!(o.contains("✗ Not undone: those files changed since the turn") && o.contains("a.txt") && o.contains("`/diff`"), "{o}");
        assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "my own edit\n");
    }

    #[tokio::test]
    async fn new_and_resume_without_a_saved_session_say_so_plainly() {
        let mut r = rig("resume", ANSWER);
        run(&mut r, "/resume").await;
        assert!(r.out.text().contains("Nothing to resume"), "{}", r.out.text());
        run(&mut r, "/new").await;
        assert!(r.out.text().contains("✓ new session"));
    }

    #[tokio::test]
    async fn a_failed_turn_ends_in_an_error_block_with_steps_and_a_summary() {
        let mut r = rig("fail", "printf 'FAIL:segmentation fault\\n'");
        let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
        assert_eq!(report.status, TurnStatus::Failed);
        r.repl.finish(&report, elapsed);
        let e = r.err.text();
        assert!(e.contains("✗ The turn failed") && e.contains("segmentation fault") && e.contains("`jev-code doctor`"), "{e}");
        assert!(e.contains("✗ claude"), "the summary marks the failure: {e}");
        assert_eq!(r.out.text(), "");
    }

    #[tokio::test]
    async fn ctrl_c_stops_the_turn_and_the_session_carries_on() {
        let mut r = rig("ctrlc", "printf 'TOOL:Edit a.txt\\n'; sleep 30; printf 'FIN:late\\n'");
        let mut int = Presses { pending: vec![Duration::from_millis(300)] };
        let TurnEnd::Done(report, _) = run_turn(&mut r.repl.engine, &r.repl.console, "do it", &mut int).await else { panic!("aborted") };
        assert_eq!(report.status, TurnStatus::Cancelled);
        r.repl.finish(&report, Duration::from_secs(3));
        let e = r.err.text();
        assert!(e.contains("■ stopped · 3s"), "{e}");
        assert!(!e.contains("cancelled") && !e.contains("[cancelled]"), "{e}");
        // The same engine takes the next prompt.
        let mut ok = rig("ctrlc-next", ANSWER);
        let mut none = Presses { pending: vec![] };
        let TurnEnd::Done(next, _) = run_turn(&mut ok.repl.engine, &ok.repl.console, "again", &mut none).await else { panic!() };
        assert_eq!(next.status, TurnStatus::Answered);
    }

    #[tokio::test]
    async fn ctrl_c_inside_an_approval_pager_does_not_stop_the_turn() {
        let console = Console::new(Box::new(Buf::default()), Box::new(Buf::default()), ConsoleConfig {
            out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: "/tmp".into(), stream_answers: true,
        });
        console.begin_paging();
        // Two presses: the first while the pager is open, the second after it closed.
        let mut int = Presses { pending: vec![Duration::from_millis(20), Duration::from_millis(200)] };
        let started = Instant::now();
        let closer = console.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            closer.end_paging();
        });
        tokio::time::timeout(Duration::from_secs(5), press_outside_pager(&mut int, &console)).await.expect("the second press counts");
        assert!(started.elapsed() >= Duration::from_millis(200), "the press inside the pager was ignored: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_second_ctrl_c_while_stopping_gives_up_waiting() {
        let mut r = rig("ctrlc2", "printf 'TOOL:Edit a.txt\\n'; sleep 30");
        let mut int = Presses { pending: vec![Duration::from_millis(200), Duration::from_millis(50)] };
        // The engine would stop on the first press; the second arrives while it is stopping
        // only if stopping takes a while, so drive it with a UI-independent check: either
        // outcome is a clean end, and an abort must not hang.
        let end = tokio::time::timeout(Duration::from_secs(10), run_turn(&mut r.repl.engine, &r.repl.console, "do it", &mut int)).await;
        assert!(matches!(end, Ok(TurnEnd::Done(..)) | Ok(TurnEnd::Aborted)), "must end promptly");
    }

    #[tokio::test]
    async fn one_shot_prints_only_the_answer_on_stdout_and_exits_zero() {
        let mut r = rig("oneshot", ANSWER);
        r.repl = Repl::new(
            r.repl.engine,
            Console::new(
                Box::new(r.out.clone()),
                Box::new(r.err.clone()),
                ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
            ),
            None,
            false,
        );
        // run_turn needs the engine's Jev call to fail fast and route without judgment.
        let mut int = Presses { pending: vec![] };
        let code = r.repl.one_shot("change a", &mut int).await;
        assert_eq!(code, 0);
        assert_eq!(r.out.text(), "the answer\n");
        let e = r.err.text();
        assert!(e.contains("⏺ Edit a.txt") && e.contains("✓ claude"), "{e}");
        assert!(!e.contains("the answer"), "the answer is not repeated on stderr: {e}");
    }

    #[tokio::test]
    async fn json_output_is_one_object_on_stdout_and_progress_stays_on_stderr() {
        let mut r = rig("oneshot-json", ANSWER);
        r.repl = Repl::new(
            r.repl.engine,
            Console::new(
                Box::new(r.out.clone()),
                Box::new(r.err.clone()),
                ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
            ),
            None,
            false,
        );
        let mut int = Presses { pending: vec![] };
        let code = r.repl.one_shot_as("change a", crate::headless::OutputFormat::Json, &mut int).await;
        assert_eq!(code, 0);
        let out = r.out.text();
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(out.starts_with("{\"status\":\"answered\",\"exit_code\":0,\"answer\":\"the answer\"") && out.contains("\"files\":["), "{out}");
        assert!(r.err.text().contains("⏺ Edit a.txt"), "{}", r.err.text());
    }

    #[tokio::test]
    async fn every_runs_the_prompt_again_until_the_times_are_done() {
        use crate::schedule::{Repeat, Schedule};
        let mut r = rig("repeat", ANSWER);
        r.repl = Repl::new(
            r.repl.engine,
            Console::new(
                Box::new(r.out.clone()),
                Box::new(r.err.clone()),
                ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
            ),
            None,
            false,
        );
        let waits = Arc::new(Mutex::new(Vec::new()));
        let seen = waits.clone();
        let mut pause: headless::Pause = Box::new(move |d| {
            seen.lock().unwrap().push(d);
            Box::pin(async {})
        });
        let repeat = Repeat { schedule: Schedule::Every(std::time::Duration::from_secs(600)), times: Some(3) };
        let mut int = Presses { pending: vec![] };
        let code = r.repl.repeat_shot("change a", crate::headless::OutputFormat::Json, &repeat, &mut int, &mut pause).await;
        assert_eq!(code, 0);
        assert_eq!(r.out.text().lines().count(), 3, "one JSON line per run: {}", r.out.text());
        assert_eq!(*waits.lock().unwrap(), vec![std::time::Duration::from_secs(600); 2], "it waits between runs, not after the last");
        assert!(r.err.text().contains("run 2 answered; next in 600s"), "{}", r.err.text());
    }

    /// A prompt through the REPL, routed without Jev (the test router has no server).
    async fn send(r: &mut Rig, line: &str) {
        let mut int = Presses { pending: vec![] };
        let mut io = Io { interrupt: &mut int, confirm: None };
        r.repl.handle(crate::commands::parse(line), &mut io).await;
    }

    #[tokio::test]
    async fn cache_and_effort_commands_read_and_set_the_session() {
        let mut r = rig("cache-effort", ANSWER);
        run(&mut r, "/cache").await;
        assert!(r.out.text().contains("No cache data yet"), "{}", r.out.text());
        run(&mut r, "/effort").await;
        assert!(r.out.text().contains("effort: default"), "{}", r.out.text());
        run(&mut r, "/effort high").await;
        assert!(r.out.text().contains("effort: high") && r.repl.engine.effort().to_string() == "high", "{}", r.out.text());
        send(&mut r, "change a").await;
        r.out.clear();
        run(&mut r, "/cache").await;
        let o = r.out.text();
        assert!(o.contains("Cache by provider") && o.contains("claude") && o.contains("hit  50%") && o.contains("Warm now"), "{o}");
        r.out.clear();
        run(&mut r, "/cost").await;
        assert!(r.out.text().contains("Cache by provider"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn cost_context_copy_and_export_read_the_session() {
        let mut r = rig("session-cmds", ANSWER);
        run(&mut r, "/cost").await;
        run(&mut r, "/context").await;
        run(&mut r, "/export").await;
        let o = r.out.text();
        assert!(o.contains("No turns in this session yet") && o.contains("No turn has run yet") && o.contains("Nothing to export yet"), "{o}");
        send(&mut r, "change a").await;
        r.out.clear();
        run(&mut r, "/cost").await;
        let o = r.out.text();
        assert!(o.contains("This session") && o.contains("1 turn") && o.contains("in     2K (cached     1K)"), "{o}");
        r.out.clear();
        run(&mut r, "/context").await;
        let o = r.out.text();
        assert!(o.contains("2K of") && o.contains("turns       1"), "{o}");
        r.out.clear();
        run(&mut r, "/export session.md").await;
        assert!(r.out.text().contains("✓ saved 1 turns to"), "{}", r.out.text());
        let md = std::fs::read_to_string(r.dir.join("session.md")).unwrap();
        assert!(md.contains("> change a") && md.contains("the answer"), "{md}");
        r.out.clear();
        run(&mut r, "/export session.md").await;
        assert!(r.out.text().contains("exists already"), "an export never overwrites: {}", r.out.text());
        assert_eq!(r.repl.log().last_answer(), Some("the answer"));
        run(&mut r, "/new").await;
        assert!(r.repl.log().is_empty(), "a new session starts a new log");
    }

    #[tokio::test]
    async fn a_shell_command_shows_its_output_and_goes_with_the_next_message() {
        // The fake model echoes what it was sent, so the note can be seen arriving.
        let mut r = rig("bang", "read -r first; printf 'FIN:%s\\n' \"$first\"");
        run(&mut r, "!echo hello-from-shell").await;
        let o = r.out.text();
        assert!(o.contains("hello-from-shell") && o.contains("✓ exit 0") && o.contains("goes with your next message"), "{o}");
        assert_eq!(r.repl.carry.len(), 1);
        assert!(r.repl.with_carry("now fix it").starts_with("I ran `echo hello-from-shell` in the shell; it exited 0."));
        assert!(r.repl.with_carry("now fix it").ends_with("---\n\nnow fix it"));
        send(&mut r, "now fix it").await;
        assert!(r.repl.carry.is_empty(), "the note went with the message");
        assert_eq!(r.repl.log().turns()[0].prompt, "now fix it", "the log keeps what the person typed");
    }

    #[tokio::test]
    async fn a_failed_turn_keeps_the_notes_for_the_next_message() {
        let mut r = rig("bang-fail", "printf 'FAIL:segmentation fault\\n'");
        run(&mut r, "!echo kept").await;
        send(&mut r, "use it").await;
        assert_eq!(r.repl.carry.len(), 1, "the shell output still waits for a turn that answers");
    }

    #[tokio::test]
    async fn why_config_and_terminal_setup_print_their_screens() {
        let mut r = rig("why", ANSWER);
        run(&mut r, "/why").await;
        assert!(r.out.text().contains("No turn has been routed yet"), "{}", r.out.text());
        send(&mut r, "change a").await;
        r.out.clear();
        run(&mut r, "/why").await;
        let o = r.out.text();
        assert!(o.contains("Last route") && o.contains("claude") && o.contains("setting"), "{o}");
        r.out.clear();
        run(&mut r, "/config").await;
        let o = r.out.text();
        assert!(o.contains("mode") && o.contains("JEV_API_KEY") && o.contains("permissions"), "{o}");
        assert!(!o.contains("sk-"), "no key value is ever shown");
        r.out.clear();
        run(&mut r, "/terminal-setup").await;
        assert!(r.out.text().contains("Shift-Enter for a new line"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/mcp").await;
        assert!(r.out.text().contains("MCP servers"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn compact_starts_fresh_with_the_summary_waiting() {
        let mut r = rig("compact", "printf 'FIN:SUMMARY of it all\\n'");
        run(&mut r, "/compact").await;
        assert!(r.out.text().contains("Nothing to compact yet"), "{}", r.out.text());
        send(&mut r, "first").await;
        let mode = r.repl.engine.mode();
        r.out.clear();
        run(&mut r, "/compact keep names").await;
        let o = r.out.text();
        assert!(o.contains("✓ compacted 1 turn"), "{o}");
        assert_eq!(r.repl.engine.mode(), mode, "the mode is put back after the read-only summary");
        assert!(r.repl.log().is_empty());
        assert!(r.repl.with_carry("next").starts_with("Summary of our conversation so far") && r.repl.with_carry("next").contains("SUMMARY of it all"));
    }

    #[tokio::test]
    async fn review_runs_read_only_and_puts_the_mode_back() {
        let mut r = rig("review", ANSWER);
        run(&mut r, "/mode auto").await;
        send(&mut r, "/review").await;
        assert!(r.err.text().contains("/review runs in plan mode"), "{}", r.err.text());
        assert_eq!(r.repl.engine.mode(), crate::mode::Mode::Auto);
        assert_eq!(r.repl.log().turns()[0].prompt, "/review");
    }

    #[test]
    fn only_a_long_turn_rings_the_bell_unless_it_is_off() {
        assert!(bell_after(Duration::from_secs(20), None));
        assert!(!bell_after(Duration::from_secs(3), None));
        assert!(!bell_after(Duration::from_secs(20), Some("off")));
        assert!(bell_after(Duration::from_secs(20), Some("on")));
    }

    #[test]
    fn page_without_a_usable_pager_says_so() {
        // A program that does not exist cannot page; the caller then prints the text itself.
        std::env::set_var("PAGER", "/nonexistent/pager-for-test");
        assert!(!page("text"));
        std::env::remove_var("PAGER");
    }

    #[tokio::test]
    async fn permissions_lists_adds_removes_and_adds_folders() {
        let mut r = rig("perms", ANSWER);
        run(&mut r, "/permissions").await;
        let o = r.out.text();
        assert!(o.contains("Permissions") && o.contains("none yet"), "{o}");
        r.out.clear();

        run(&mut r, "/permissions allow Bash(cargo test:*)").await;
        run(&mut r, "/permissions allow Bash(cargo test:*)").await;
        let o = r.out.text();
        assert!(o.contains("✓ allowed Bash(cargo test:*)") && o.contains("already allowed"), "{o}");
        r.out.clear();

        std::fs::create_dir_all(r.dir.join("data")).unwrap();
        run(&mut r, "/permissions dir data").await;
        run(&mut r, "/permissions dir nowhere").await;
        let o = r.out.text();
        assert!(o.contains("✓ added folder") && o.contains("data") && o.contains("⚠"), "{o}");
        r.out.clear();

        run(&mut r, "/permissions").await;
        let o = r.out.text();
        assert!(o.contains("Bash(cargo test:*)") && o.contains("additional folders"), "{o}");
        let file = std::fs::read_to_string(r.dir.join(".claude/settings.local.json")).unwrap();
        assert!(file.contains("Bash(cargo test:*)") && file.contains("additionalDirectories"), "{file}");
        r.out.clear();

        run(&mut r, "/permissions remove Bash(cargo test:*)").await;
        run(&mut r, "/permissions remove Bash(cargo test:*)").await;
        let o = r.out.text();
        assert!(o.contains("✓ removed Bash(cargo test:*)") && o.contains("is not in the project's rules"), "{o}");
        r.out.clear();

        run(&mut r, "/permissions allow nonsense(").await;
        assert!(r.out.text().contains("not a permission rule"), "{}", r.out.text());
        std::fs::write(r.dir.join(".claude/settings.local.json"), "{ broken").unwrap();
        r.out.clear();
        run(&mut r, "/permissions allow WebSearch").await;
        assert!(r.out.text().contains("not valid JSON") && r.out.text().contains("left as it is"), "{}", r.out.text());
        assert_eq!(std::fs::read_to_string(r.dir.join(".claude/settings.local.json")).unwrap(), "{ broken");
    }

    const ECHO_PROMPT: &str = "printf 'FIN:%s\\n' \"$(printf '%s' \"$FAKE_PROMPT\" | tr '\\n' ' ')\"";

    fn write_ext(r: &Rig, rel: &str, text: &str) {
        let path = r.dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[tokio::test]
    async fn a_custom_command_runs_its_prompt_with_the_arguments() {
        let mut r = rig("custom-cmd", ECHO_PROMPT);
        write_ext(&r, ".jev/commands/shout.md", "---\ndescription: Shout\n---\nSHOUT about $ARGUMENTS now");
        assert_eq!(run(&mut r, "/shout the parser").await, Flow::Continue);
        assert!(r.out.text().contains("SHOUT about the parser now"), "{}", r.out.text());
        assert_eq!(r.repl.log().turns()[0].prompt, "/shout the parser");
        // Still an unknown command when no file defines it.
        r.out.clear();
        run(&mut r, "/nothing-here").await;
        assert!(r.out.text().contains("unknown command /nothing-here"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn a_recipe_runs_its_steps_in_order_and_lists_with_recipe() {
        let mut r = rig("recipe", ECHO_PROMPT);
        write_ext(&r, ".jev/recipes/two.md", "---\ndescription: Two steps\n---\n## First\nstep-one for $1\n## Second\nstep-two");
        run(&mut r, "/recipe").await;
        assert!(r.out.text().contains("/two") && r.out.text().contains("2 steps"), "{}", r.out.text());
        r.out.clear();
        run(&mut r, "/recipe two alpha").await;
        let o = r.out.text();
        assert!(o.contains("step 1/2: First") && o.contains("step-one for alpha") && o.contains("step-two"), "{o}");
        let turns = r.repl.log().turns();
        assert_eq!((turns.len(), turns[0].prompt.as_str(), turns[1].prompt.as_str()), (2, "/two alpha [1/2]", "/two alpha [2/2]"));
        r.out.clear();
        run(&mut r, "/recipe missing").await;
        assert!(r.out.text().contains("no recipe named missing"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn a_recipe_stops_at_the_first_step_that_fails() {
        let mut r = rig("recipe-stop", "printf 'FAIL:boom\\n'; exit 1");
        write_ext(&r, ".jev/recipes/two.md", "## A\none\n## B\ntwo");
        run(&mut r, "/recipe two").await;
        assert_eq!(r.repl.log().turns().len(), 1, "the second step was not sent");
    }
}
