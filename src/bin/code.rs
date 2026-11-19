//! `jev-code`: a routed coding agent. Each turn runs one provider's coding CLI (Claude Code,
//! Codex, Gemini, DeepSeek through Claude Code, Grok) in your project with tools on. Jev says
//! what kind of task the turn is; the router picks the best model among the providers you can
//! really use, catches it up on what the others did, and fails over to the next one on a limit
//! or an error.
//!
//! This file only wires the terminal to the library: arguments (`jev_router::cli`), the line
//! editor (`lineedit`), output (`console`), commands and turns (`repl`), screens (`views`).
//! Every exit goes through `run`'s return value, so the terminal guard always runs.
//!
//!   jev-code [dir]            interactive session
//!   jev-code -p "prompt"      one turn, only the answer on stdout
//!   jev-code doctor           check the setup

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use jev_router::cli::{self, Action, Args, PromptSource};
use jev_router::commands;
use jev_router::console::{Console, ConsoleConfig};
use jev_router::envfile;
use jev_router::front::{Front, FrontMsg};
use jev_router::tui;
use jev_router::handoff;
use jev_router::lineedit::{self, Input, InterruptGuard};
use jev_router::log;
use jev_router::mode;
use jev_router::orchestrate::{self, Setup};
use jev_router::prefs::Saved;
use jev_router::perf::{Phase, ProbeSource, Timeline};
use jev_router::probecache;
use jev_router::providers::{self, ProbeCache, Runtime};
use jev_router::repl::{Confirm, Flow, Interrupt, Io, Repl, SigInt};
use jev_router::router::{Router, CODE_STRONG_THRESHOLD};
use jev_router::terminal::{self, TerminalGuard};
use jev_router::turn::Engine;
use jev_router::typesafe::TypeSafe;
use jev_router::uifmt::{self, Style};
use jev_router::views;

/// What is known about the terminals before anything is printed.
struct Term {
    stdin_tty: bool,
    stdout_tty: bool,
    out_style: Style,
    err_style: Style,
    width: usize,
}

impl Term {
    fn detect(no_color: bool) -> Self {
        let (stdin_tty, stdout_tty, stderr_tty) =
            (std::io::stdin().is_terminal(), std::io::stdout().is_terminal(), std::io::stderr().is_terminal());
        let style = |tty: bool| if no_color { Style::off() } else { Style::detect(tty) };
        // A width only means something on a terminal; elsewhere lines are never cut.
        let width = if stdout_tty || stderr_tty { terminal::width() } else { 4096 };
        Self { stdin_tty, stdout_tty, out_style: style(stdout_tty), err_style: style(stderr_tty), width }
    }

    /// Whether this is a person at a keyboard: the line editor, the banner hints and the
    /// first-run hint are for them only.
    fn interactive(&self) -> bool {
        self.stdin_tty && self.stdout_tty
    }

    /// The live status line needs a terminal on stderr that takes escape sequences.
    fn live(&self) -> bool {
        std::io::stderr().is_terminal() && self.err_style.enabled
    }
}

/// `stream_answers` is false for `-p`, which also never asks: a one-shot run behaves the
/// same on a terminal and in a pipe. A session on a terminal hands the screen to the
/// interactive front end (`front`), which draws everything and asks about each tool call on
/// a card.
fn console_for(term: &Term, project: &Path, stream_answers: bool, front: Option<Front>) -> Console {
    let console = Console::stdio(ConsoleConfig {
        out_style: term.out_style,
        err_style: term.err_style,
        width: term.width,
        live: term.live() && front.is_none(),
        project: project.to_path_buf(),
        stream_answers,
    });
    if let Some(front) = front {
        return console.with_front(front);
    }
    if term.interactive() && stream_answers {
        let console = console.with_key_reader(jev_router::console::terminal_keys());
        // What is typed during a turn shows on the status line: without one it would be blind.
        if term.live() {
            console.with_type_ahead()
        } else {
            console
        }
    } else {
        console
    }
}

/// The front end's end of the channel from the session.
type FrontRx = tokio::sync::mpsc::UnboundedReceiver<FrontMsg>;

#[tokio::main]
async fn main() {
    let code = run().await;
    // Every guard is dropped by now. Leave without waiting for a thread blocked on stdin.
    std::process::exit(code);
}

async fn run() -> i32 {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("jev-code: {e}\n{}", cli::USAGE);
            return 2;
        }
    };
    match args.action {
        Action::Help => {
            println!("{}", cli::HELP);
            return 0;
        }
        Action::Version => {
            println!("jev-code {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        Action::Ext(action) => return ext_command(&action, args.no_color),
        Action::Update { yes } => return update_command(yes),
        _ => {}
    }
    let term = Term::detect(args.no_color);
    let _terminal = TerminalGuard::install();
    let state_dir = log::state_dir();
    terminal::install_panic_hook(state_dir.as_ref().map(|d| d.join("panic.log")));
    terminal::restore_on_termination();

    let dir = match project_dir(&args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    apply_layered_settings(&args, &dir);
    // A person at a terminal gets the interactive screen; everything else prints plainly.
    let (front, front_rx) = match args.action {
        Action::Doctor | Action::Print(_) => (None, None),
        _ if args.startup_profile => (None, None),
        _ if term.interactive() && std::env::var_os("JEV_CLASSIC_UI").is_none() => {
            let (front, rx) = Front::channel();
            (Some(front), Some(rx))
        }
        _ => (None, None),
    };
    let console = console_for(&term, &dir, !matches!(args.action, Action::Print(_)), front);
    match &args.action {
        Action::Doctor => doctor(&dir, &console).await,
        Action::Print(source) => print_mode(&args, source, &dir, &term, console, state_dir).await,
        _ => session(&args, &dir, &term, console, state_dir, front_rx).await,
    }
}

/// Settings from the config files and the chosen profile reach the process environment
/// before anything reads one (`/settings` shows them). Problems are shown there, not here.
fn apply_layered_settings(args: &Args, dir: &Path) {
    if let Some(profile) = &args.profile {
        std::env::set_var("JEV_PROFILE", profile);
    }
    let profile = std::env::var("JEV_PROFILE").ok().filter(|p| !p.is_empty());
    let places = jev_router::ext::Places::from_env(dir);
    jev_router::layers::Layered::load(&places, profile.as_deref()).apply_to_process();
}

/// `jev-code update [--yes]`: the same as `/update`, from the shell. Asks on the terminal unless
/// `--yes`; without a terminal to ask on, only `--yes` updates. Exit code: 0 updated or already
/// the newest, 1 refused or failed, 2 not confirmed.
fn update_command(yes: bool) -> i32 {
    use jev_router::selfupdate::{self, Plan};
    use std::io::{BufRead, IsTerminal, Write};
    let dir = match selfupdate::source_dir() {
        Ok(dir) => dir,
        Err(why) => {
            eprintln!("cannot update: {why}");
            return 1;
        }
    };
    eprintln!("checking {} for a new version...", dir.display());
    let plan = match selfupdate::check(&dir) {
        Ok(facts) => selfupdate::plan(&facts, selfupdate::running_version()),
        Err(why) => Plan::Refused(why),
    };
    for line in selfupdate::plan_text(&plan) {
        println!("{line}");
    }
    let pull = match plan {
        Plan::UpToDate { .. } => return 0,
        Plan::Refused(_) => return 1,
        Plan::Update { pull, .. } => pull,
    };
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("not updated: run it in a terminal to be asked, or pass --yes");
            return 2;
        }
        eprint!("{}", if pull { "Pull it and reinstall jev-code now? [y/N] " } else { "Reinstall jev-code from the checkout now? [y/N] " });
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            eprintln!("left as it is");
            return 2;
        }
    }
    match selfupdate::apply(&dir, pull, &mut |line| println!("  {line}")) {
        Ok(version) => {
            println!("jev-code {version} is installed");
            0
        }
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// `jev-code ext ...`: no providers and no terminal setup, only the config folder.
fn ext_command(action: &jev_router::ext::cli::ExtAction, no_color: bool) -> i32 {
    let style = if no_color { Style::off() } else { Style::detect(std::io::stdout().is_terminal()) };
    let Some(user) = jev_router::ext::config_dir(|n| std::env::var(n).ok()) else {
        eprintln!("jev-code: no config folder for extensions (HOME is not set; or set JEV_CODE_CONFIG_DIR)");
        return 1;
    };
    let outcome = jev_router::ext::cli::run(action, &user, style);
    println!("{}", outcome.text);
    outcome.code
}

/// The project directory, or the message that says why it cannot be used.
fn project_dir(args: &Args) -> Result<PathBuf, String> {
    let given = match &args.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir().map_err(|e| format!("jev-code: cannot tell the current directory: {e}"))?,
    };
    given.canonicalize().map_err(|e| {
        let style = Style::detect(std::io::stderr().is_terminal());
        let steps = ["Check the path, or start jev-code without one to use the current directory".to_string()];
        uifmt::problem_block("Can't open the project folder", &[format!("{}: {e}", given.display())], &steps, style)
    })
}

// ------------------------------------------------------------------------ setup

/// The Jev client, or the message that says how to get a key.
fn jev_client(style: Style) -> Result<TypeSafe, String> {
    TypeSafe::from_env().map_err(|_| {
        let steps = [
            "Add JEV_API_KEY=... to ~/.jev-router.env (copy .jev-router.env.example and `chmod 600` it)".to_string(),
            "Or export JEV_API_KEY (or TYPESAFE_API_KEY) in your shell".to_string(),
            "Then check everything with `jev-code doctor`".to_string(),
        ];
        uifmt::problem_block("No Jev key found", &["jev-code asks Jev to judge each task, so it needs a key.".to_string()], &steps, style)
    })
}

/// Whether the provider probe may reuse the last run's answer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeMode {
    /// Sessions and one-shot turns: a fresh cached answer is used at once and checked in the
    /// background, so the prompt never waits for the provider CLIs.
    Cached,
    /// `doctor` reports what is true now.
    Live,
}

/// The probe cache of this machine, unless the user turned it off.
fn probe_cache() -> Option<ProbeCache> {
    if probecache::disabled(envfile::lookup("JEV_PROBE_CACHE").as_deref()) {
        return None;
    }
    Some(ProbeCache { path: probecache::default_path()?, fingerprint: probecache::fingerprint(&|name| envfile::lookup(name)) })
}

/// Probe the providers, with a one-line notice on a terminal when it has to ask the CLIs
/// (that can take seconds); a cached answer needs no notice.
async fn start_runtime(console: &Console, mode: ProbeMode) -> (Runtime, ProbeSource) {
    let cache = (mode == ProbeMode::Cached).then(probe_cache).flatten();
    let warm = cache.as_ref().is_some_and(|c| probecache::load(&c.path, log::now_ms(), c.fingerprint).is_some());
    let live = !warm && console.err_style().enabled && std::io::stderr().is_terminal();
    if live {
        eprint!("{}", console.err_style().dim("checking providers..."));
    }
    let started = Runtime::start_cached(cache).await;
    if live {
        eprint!("{}", uifmt::clear_line());
    }
    started
}

/// Settings come from the environment or `~/.jev-router.env`, like every other setting.
/// `can_ask`: the console can put an approval question to a person
/// ([`Console::can_ask`]); otherwise the CLIs run without the host prompt protocol.
/// `session`: what the last session left (`/mode`, `/orchestrate`, `/provider`) for an
/// interactive session; a one-shot run nobody can answer passes `None` and starts from the
/// flags and the environment alone.
fn engine_for(args: &Args, runtime: Runtime, router: Router, dir: PathBuf, can_ask: bool, session: Option<Saved>) -> Engine {
    let saved = session.unwrap_or_default();
    let mode = mode::start(args.mode, saved.mode, envfile::lookup("CODE_PERMISSION_MODE").as_deref());
    let routing = saved.routing.unwrap_or_else(|| commands::routing_enabled(envfile::lookup("JEV_PROVIDERS").as_deref()));
    let orchestrate = orchestrate::start(saved.orchestrate, envfile::lookup("JEV_ORCHESTRATE").as_deref(), session.is_some());
    let hooks = std::sync::Arc::new(jev_router::ext::hooks::Hooks::load(&jev_router::ext::Places::from_env(&dir)));
    let engine = Engine::new(runtime, router, dir)
        .with_backend_factory(Box::new(providers::run_backends(can_ask)))
        .with_enabled(routing)
        .with_mode(mode)
        .with_orchestrate(orchestrate)
        .with_orch_setup(Setup::detect())
        .with_hooks(hooks)
        .with_downgrade_after(envfile::lookup_f64("DOWNGRADE_AFTER", 2.0) as u32);
    match &args.pin {
        Some(pin) => {
            let mut engine = engine;
            engine.set_pin(pin.clone());
            engine
        }
        None => engine,
    }
}

// ----------------------------------------------------------------------- doctor

/// `jev-code doctor`: the report on stdout; exit 1 when something is a Problem.
async fn doctor(dir: &Path, console: &Console) -> i32 {
    let (runtime, _) = start_runtime(console, ProbeMode::Live).await;
    let inputs = jev_router::doctor::gather(&runtime, dir).await;
    let report = jev_router::doctor::diagnose(&inputs);
    console.out(&views::doctor_text(&report, console.out_style(), console.width()));
    i32::from(views::has_problem(&report))
}

// --------------------------------------------------------------------- one-shot

/// `jev-code -p`: one turn. The answer alone on stdout; exit 0 answered, 1 failed, 130 stopped.
async fn print_mode(args: &Args, source: &PromptSource, dir: &Path, term: &Term, console: Console, state_dir: Option<PathBuf>) -> i32 {
    let prompt = match source {
        PromptSource::Text(t) => t.clone(),
        PromptSource::Stdin => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                eprintln!("jev-code: cannot read the prompt from stdin: {e}");
                return 2;
            }
            text
        }
    };
    if prompt.trim().is_empty() {
        eprintln!("jev-code: no prompt given (nothing on stdin)\n{}", cli::USAGE);
        return 2;
    }
    let router = match jev_client(term.err_style) {
        Ok(ts) => router_for(ts),
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let (runtime, _) = start_runtime(&console, ProbeMode::Cached).await;
    // A one-shot starts a fresh conversation and leaves the saved session alone, unless -c asks to continue it.
    let persist = if args.resume { state_dir.clone() } else { None };
    // The privacy rules, budget and failover settings apply to a one-shot too: they are read from
    // the state folder even when the session itself is not kept.
    let engine = engine_for(args, runtime, router, dir.to_path_buf(), console.can_ask(), None).with_persistence_and_settings(persist, state_dir.as_deref());
    let mut repl = Repl::new(engine, console.clone(), std::env::var("HOME").ok(), false);
    repl.enforce_team_limit();
    if args.resume {
        resume_quietly(&mut repl, &console);
    }
    let mut sigint = match SigInt::install() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("jev-code: cannot listen for Ctrl-C: {e}");
            return 1;
        }
    };
    match &args.repeat {
        Some(repeat) => repl.repeat_shot(&prompt, args.format, repeat, &mut sigint, &mut jev_router::repl::sleeper()).await,
        None => repl.one_shot_as(&prompt, args.format, &mut sigint).await,
    }
}

fn router_for(ts: TypeSafe) -> Router {
    // The strong threshold only matters with routing off (the Claude-only tier rule).
    Router::new(ts, 1.0).with_strong_threshold(envfile::lookup_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD))
}

/// `-c`: load the saved session and say so on stderr; nothing to continue is not an error.
fn resume_quietly(repl: &mut Repl, console: &Console) {
    let style = console.err_style();
    match repl.engine.resume() {
        Ok(info) => console.err(&format!("  {}", views::resumed_line(&info, log::now_ms(), style))),
        Err(e) => console.err(&format!("  {}", style.dim(&format!("starting fresh: {e}")))),
    }
}

// ---------------------------------------------------------------------- session

/// Lets `/undo` ask its question through the line editor.
struct EditorConfirm<'a>(&'a mut lineedit::LineEditor);

impl Confirm for EditorConfirm<'_> {
    fn confirm(&mut self, question: &str) -> bool {
        tokio::task::block_in_place(|| lineedit::confirm(self.0, question))
    }
}

async fn session(args: &Args, dir: &Path, term: &Term, console: Console, state_dir: Option<PathBuf>, front_rx: Option<FrontRx>) -> i32 {
    let mut timeline = Timeline::start(args.trace);
    let router = match timeline.time(Phase::JevClient, || jev_client(term.err_style)) {
        Ok(ts) => router_for(ts),
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let began = timeline.now();
    let (runtime, probes) = start_runtime(&console, ProbeMode::Cached).await;
    timeline.set_probe_source(probes);
    timeline.record(Phase::ProviderProbe, began);
    let saved = timeline.time(Phase::SavedState, || state_dir.as_deref().map(Saved::load).unwrap_or_default());
    let home = std::env::var("HOME").ok();
    let mut repl = timeline.time(Phase::Engine, || {
        let engine = engine_for(args, runtime, router, dir.to_path_buf(), console.can_ask(), Some(saved)).with_persistence(state_dir.clone());
        // The interactive screen shows text as it arrives; plain output keeps it until it is final.
        let engine = if front_rx.is_some() { engine.with_streaming() } else { engine };
        Repl::new(engine, console.clone(), home.clone(), term.stdout_tty).with_state_dir(state_dir.clone())
    });
    repl.enforce_team_limit();
    timeline.time(Phase::Banner, || startup(&mut repl, args, term, &console, state_dir.as_deref(), home.as_deref()));
    if args.startup_profile {
        console.out(&timeline.report());
        return 0;
    }
    recheck_slow_providers(&mut repl, &console);

    let mut sigint = match SigInt::install() {
        Ok(s) => s,
        Err(e) => {
            console.err(&format!("jev-code: cannot listen for Ctrl-C: {e}"));
            return 1;
        }
    };
    if let Some(rx) = front_rx {
        // The front end saves the session when it ends.
        let opts = tui::run::Options { style: term.out_style, history: state_dir.as_deref().map(lineedit::history_path) };
        return tui::run::run(repl, rx, opts).await;
    }
    let code = if term.interactive() {
        edit_loop(&mut repl, term, &console, state_dir.as_deref(), &mut sigint).await
    } else {
        pipe_loop(&mut repl, &mut sigint).await
    };
    if let Err(e) = repl.engine.save_now() {
        console.err(&format!("  could not save the session: {e}"));
    }
    code
}

/// A provider whose probe timed out at startup (a lazy wrapper's first run) is asked again in
/// the background; the banner above did not wait for it. The runtime learns the answer in
/// [`settle_late_probes`], and the notice is printed through the console, so it never lands in
/// the middle of a turn's output or of the line being typed.
fn recheck_slow_providers(repl: &mut Repl, console: &Console) {
    let notices = console.clone();
    repl.engine.runtime.retry_slow_probes(move |a| notices.notify(&format!("{} is ready now", a.provider)));
}

/// Before a prompt is shown or a line is run: apply what the background re-check found and
/// print the notices held back for the prompt. Never mid-turn, so routing cannot change under one.
fn settle_late_probes(repl: &mut Repl) {
    repl.engine.runtime.absorb_late();
    repl.console.flush_notices();
}

/// The banner, the offer to resume (or the resume itself), and the one-time hint.
fn startup(repl: &mut Repl, args: &Args, term: &Term, console: &Console, state_dir: Option<&Path>, home: Option<&str>) {
    let (style, width, now) = (term.out_style, term.width, log::now_ms());
    let rows = views::avail_rows(&repl.engine.runtime, now);
    console.out(&views::banner(repl.engine.dir(), home, repl.engine.enabled(), &rows, style, width));
    if let Some(notice) = state_dir.and_then(|d| jev_router::inflight::recovery_notice(d, repl.engine.dir())) {
        console.out(&style.yellow(&notice));
    }
    if args.resume {
        match repl.engine.resume() {
            Ok(info) => console.out(&views::resumed_line(&info, now, style)),
            Err(e) => console.out(&style.dim(&format!("no session to continue ({e}); starting fresh"))),
        }
    } else if let Some(info) = state_dir.and_then(|d| handoff::saved_session_info_in(d, repl.engine.dir(), now)) {
        console.out(&views::previous_session_line(&info, now, style));
    }
    if term.interactive() {
        if repl.console.front().is_none() {
            console.out(&uifmt::truncate_to_width(&style.dim("/help commands  ·  Shift+Tab mode"), width));
        }
        if let Some(hint) = views::first_run_hint_once(&repl.engine.runtime.avail, state_dir) {
            for line in hint.lines() {
                console.out(&style.dim(line));
            }
            console.out(&style.dim("Run /doctor for the full check and the exact commands."));
        }
    }
}

/// The interactive loop: the line editor, one message or command at a time.
async fn edit_loop(repl: &mut Repl, term: &Term, console: &Console, state_dir: Option<&Path>, sigint: &mut SigInt) -> i32 {
    let history = state_dir.map(lineedit::history_path);
    // Shift-Tab changes this while a prompt is open; it is read back after every line.
    let shared_mode = mode::SharedMode::new(repl.engine.mode());
    let mut editor = match lineedit::build_editor(term.out_style, history.as_deref(), shared_mode.clone()) {
        Ok(e) => e,
        Err(e) => {
            console.err(&format!("jev-code: cannot start the line editor: {}", lineedit::describe(&e)));
            return pipe_loop(repl, sigint).await;
        }
    };
    if let Some(helper) = editor.helper_mut() {
        helper.set_project(repl.engine.dir());
    }
    let mut guard = InterruptGuard::default();
    loop {
        settle_late_probes(repl);
        shared_mode.set(repl.engine.mode());
        let label = views::prompt_label(repl.engine.pin(), repl.engine.enabled());
        let plain = views::plain_prompt(label.as_deref());
        let styled = views::styled_prompt(label.as_deref(), term.out_style);
        let ctx = views::complete_ctx(&repl.engine, log::now_ms());
        // A message left unfinished when the last turn ended comes back into the prompt.
        let draft = console.take_draft();
        let input = tokio::task::block_in_place(|| lineedit::read(&mut editor, &plain, &styled, ctx, &draft));
        // A mode picked with Shift-Tab holds even when the line was thrown away.
        if shared_mode.get() != repl.engine.mode() {
            let applied = repl.adopt_mode(shared_mode.get());
            shared_mode.set(applied);
            console.out(&format!("  mode: {}", views::mode_line(applied)));
        }
        match input {
            Input::Line(line) => {
                settle_late_probes(repl);
                guard.reset();
                lineedit::remember(&mut editor, &line, history.as_deref());
                let mut flow = run_line(repl, &mut editor, sigint, &line).await;
                // Messages typed during the turn run next, in order, until one stops the loop.
                while flow == Flow::Continue {
                    let Some(next) = console.take_queued() else { break };
                    console.out(&format!("{} {}", term.out_style.cyan("›"), next.replace('\n', "\n  ")));
                    lineedit::remember(&mut editor, &next, history.as_deref());
                    flow = run_line(repl, &mut editor, sigint, &next).await;
                }
                match flow {
                    Flow::Continue => {}
                    Flow::Quit => return 0,
                    Flow::Aborted => return 130,
                }
            }
            Input::Interrupted => match guard.on_interrupt(Instant::now()) {
                lineedit::Interrupt::Warn => console.err(&format!("\n{}", term.err_style.dim(lineedit::EXIT_HINT))),
                lineedit::Interrupt::Exit => return 0,
            },
            Input::Eof => {
                console.out("");
                return 0;
            }
            Input::Failed(message) => {
                console.err(&format!("jev-code: {message}"));
                return 1;
            }
        }
    }
}

/// One line from the person (typed, or queued during a turn), with `/undo` able to ask.
async fn run_line(repl: &mut Repl, editor: &mut lineedit::LineEditor, sigint: &mut SigInt, line: &str) -> Flow {
    let mut confirm = EditorConfirm(editor);
    let mut io = Io { interrupt: sigint, confirm: Some(&mut confirm) };
    repl.handle(commands::parse(line), &mut io).await
}

/// Lines from a pipe: one message or command per line, no prompt, no editing. Ctrl-C while
/// waiting for the next line ends the session.
async fn pipe_loop(repl: &mut Repl, sigint: &mut SigInt) -> i32 {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        let next = tokio::select! {
            line = lines.next_line() => line,
            _ = sigint.wait() => return 130,
        };
        let line = match next {
            Ok(Some(line)) => line,
            Ok(None) => return 0,
            Err(e) => {
                repl.console.err(&format!("jev-code: cannot read input: {e}"));
                return 1;
            }
        };
        settle_late_probes(repl);
        let mut io = Io { interrupt: sigint, confirm: None };
        match repl.handle(commands::parse(&line), &mut io).await {
            Flow::Continue => {}
            Flow::Quit => return 0,
            Flow::Aborted => return 130,
        }
    }
}
