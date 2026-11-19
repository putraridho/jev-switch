//! The interactive loop: keys, the session's messages and a timer in, the screen out.
//!
//! The session ([`Repl`]) runs each line on a task of its own, so the screen keeps reading
//! keys while a turn works: typing queues the next message, Esc and Ctrl-C stop the turn,
//! and approvals are answered on a card. Between lines the session comes back here.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, MouseEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::{execute, terminal};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::app::{Action, App};
use super::history::History;
use super::screen::Screen;
use super::view;
use crate::commands;
use crate::console::TICK;
use crate::front::{Front, FrontMsg};
use crate::lineedit;
use crate::log;
use crate::repl::{Confirm, Flow, Interrupt, Io, Repl};
use crate::uifmt::Style;
use crate::views;

/// What the loop needs besides the session.
pub struct Options {
    pub style: Style,
    /// `<state dir>/history`, when there is a state folder.
    pub history: Option<PathBuf>,
}

/// Stop requests from the screen, as the session's [`Interrupt`].
struct Stops(mpsc::UnboundedReceiver<()>);

impl Interrupt for Stops {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // A closed channel never fires: it must not look like a stop.
            if self.0.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        })
    }

    fn drain(&mut self) {
        while self.0.try_recv().is_ok() {}
    }
}

/// Yes/no questions from commands, answered on a card.
struct CardConfirm(Option<Front>);

impl Confirm for CardConfirm {
    fn confirm(&mut self, question: &str) -> bool {
        let question = question.trim_end().trim_end_matches("[y/N]").trim_end();
        self.0.as_ref().is_some_and(|f| f.confirm(question))
    }
}

/// Switch on the keyboard modes the screen needs: raw keys, bracketed paste, and the
/// keyboard protocol that reports Shift-Enter apart from Enter (terminals that do not know it
/// ignore the request).
fn keys_on() -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    pick_background();
    let mut out = std::io::stdout();
    if FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst) {
        use std::io::Write;
        out.write_all(super::screen::FULL_ON.as_bytes())?;
        out.write_all(b"\x1b[2J")?;
        out.flush()?;
        crate::terminal::note_alt_screen(true);
    }
    execute!(out, crossterm::event::EnableBracketedPaste)?;
    // Inside tmux or on a console the request could be printed instead of understood.
    if term_env().keyboard_protocol() {
        execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
        crate::terminal::note_keyboard_protocol(true);
    }
    crate::terminal::note_front_keys(true);
    Ok(())
}

/// Light or dark terminal: what `JEV_THEME` or `COLORFGBG` says, else what the terminal
/// answers (not asked inside tmux or screen, which may print the question instead).
fn pick_background() {
    let var = |k: &str| std::env::var(k).ok();
    let light = crate::uifmt::background_from_env(var("JEV_THEME").as_deref(), var("COLORFGBG").as_deref()).or_else(|| {
        let env = term_env();
        (env.multiplexer.is_none() && env.kind == crate::termenv::Kind::Normal).then(crate::bgquery::query_light).flatten()
    });
    crate::uifmt::set_light_background(light.unwrap_or(false));
}

fn term_env() -> crate::termenv::TermEnv {
    crate::termenv::detect(&|name| std::env::var(name).ok())
}

/// Put the keyboard back as a command or the shell expects it.
fn keys_off() {
    let mut out = std::io::stdout();
    if FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst) {
        use std::io::Write;
        let _ = out.write_all(crate::terminal::LEAVE_ALT_SCREEN);
        let _ = out.flush();
        crate::terminal::note_alt_screen(false);
    }
    if crate::terminal::note_keyboard_protocol(false) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, crossterm::event::DisableBracketedPaste);
    let _ = terminal::disable_raw_mode();
    crate::terminal::note_front_keys(false);
}

/// Turns the keyboard modes off when dropped, on every way out of the loop.
struct KeyModes;

impl Drop for KeyModes {
    fn drop(&mut self) {
        keys_off();
    }
}

fn width() -> usize {
    terminal::size().map(|(w, _)| usize::from(w)).unwrap_or_else(|_| crate::terminal::width())
}

/// The full-screen layout is on: the app keeps the conversation and draws the whole terminal
/// (alternate screen, chat pane, input box, full-height side panel).
static FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Full screen on a terminal, unless `JEV_INLINE=1` asks for the conversation to go into the
/// terminal's own scrollback instead.
fn fullscreen_wanted() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal() && std::env::var("JEV_INLINE").map_or(true, |v| v.is_empty() || v == "0")
}

/// Terminal columns and rows.
fn size() -> (usize, usize) {
    terminal::size().map(|(w, h)| (usize::from(w), usize::from(h))).unwrap_or_else(|_| (crate::terminal::width(), 24))
}

/// The columns text is laid out in: the chat pane when the side panel has its own column.
fn text_width(app: &App) -> usize {
    let cols = size().0;
    if app.fullscreen {
        let w = cols.saturating_sub(1).max(20);
        if w >= crate::workstation::panel::SPLIT_MIN_WIDTH { w - crate::workstation::panel::PANEL_WIDTH - 3 } else { w }
    } else {
        cols
    }
}

/// What the screen shows now.
fn frame(app: &mut App, style: Style) -> view::Frame {
    let (cols, rows) = size();
    if app.fullscreen {
        crate::terminal::set_width_cap(Some(text_width(app)));
        let (frame, pane) = view::render_full(app, cols, rows, Instant::now(), style);
        app.pane_rows = pane;
        frame
    } else {
        view::render(app, cols, Instant::now(), style)
    }
}

fn draw(screen: &mut Screen<std::io::Stdout>, app: &mut App, style: Style) {
    let f = frame(app, style);
    if app.fullscreen {
        screen.draw_full(&f);
    } else {
        screen.draw(&f);
    }
}

/// Finished output: into the conversation, or above the live area inline.
fn show(screen: &mut Screen<std::io::Stdout>, app: &mut App, text: &str, style: Style) {
    if app.fullscreen {
        app.show(text);
    } else {
        let f = frame(app, style);
        screen.print_above(text, &f);
    }
}

/// The branch and how many files differ from it, `main ±3`; `None` outside a repository.
fn git_line(dir: &std::path::Path) -> Option<String> {
    let git = |args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(dir).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let changed = git(&["status", "--porcelain"]).map_or(0, |s| s.lines().count());
    Some(if changed > 0 { format!("{branch} ±{changed}") } else { branch })
}

/// What the router row shows: each subscription that can take a turn, with how much of its
/// plan window is used, and those cooling down after a limit.
fn provider_chips(repl: &Repl) -> Vec<super::app::ProviderChip> {
    let rt = &repl.engine.runtime;
    views::avail_rows(rt, log::now_ms())
        .into_iter()
        .filter(|r| r.status == crate::uifmt::ProviderStatus::Ready)
        .map(|r| {
            let used = r.provider.parse::<crate::backend::ProviderId>().ok().and_then(|p| rt.plan_utilization(p)).map(|u| (u.clamp(0.0, 1.0) * 100.0).round() as u8);
            super::app::ProviderChip { name: r.provider, used, cooling: r.cooldown, via: r.via }
        })
        .collect()
}

/// Bring the screen's view of the session up to date (between lines).
fn refresh(app: &mut App, repl: &Repl) {
    app.route = route_label(repl);
    app.ctx = views::complete_ctx(&repl.engine, log::now_ms());
    app.git = git_line(repl.engine.dir());
    app.context = repl.context_percent();
    app.session_name = repl.engine.sessions.name().map(String::from);
    app.providers = provider_chips(repl);
    app.orchestrate = repl.engine.orchestrate();
}

/// The project's open workers, for the rows above the box.
fn worker_chips(state_dir: Option<&std::path::Path>, project: &std::path::Path) -> Vec<super::app::WorkerChip> {
    let now = log::now_ms();
    state_dir
        .map(|s| crate::delegate::open_workers(s, project))
        .unwrap_or_default()
        .into_iter()
        .map(|w| super::app::WorkerChip { id: w.worker_id, provider: w.provider, model: w.model, running: w.running, age_ms: now.saturating_sub(w.created_ms), tools: w.tools, activity: w.activity })
        .collect()
}

/// The footer's routing word: the pin, `claude only`, or `auto`.
fn route_label(repl: &Repl) -> String {
    views::prompt_label(repl.engine.pin(), repl.engine.enabled()).unwrap_or_else(|| "auto".to_string())
}


/// The next key or paste, or `None` while the terminal belongs to a command.
async fn next_event(events: &mut Option<EventStream>) -> Option<std::io::Result<Event>> {
    match events {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// The running line, or never.
async fn finished(task: &mut Option<JoinHandle<Done>>) -> Option<Done> {
    match task {
        Some(handle) => {
            let done = handle.await.ok();
            *task = None;
            done
        }
        None => std::future::pending().await,
    }
}

/// A finished line: the session back, its stop channel back, and what to do next.
type Done = (Repl, Stops, Flow);

/// Run the interactive session until the person quits. Returns the exit code.
pub async fn run(repl: Repl, mut rx: mpsc::UnboundedReceiver<FrontMsg>, opts: Options) -> i32 {
    let style = opts.style;
    FULLSCREEN.store(fullscreen_wanted(), std::sync::atomic::Ordering::SeqCst);
    let _modes = match keys_on() {
        Ok(()) => KeyModes,
        Err(e) => {
            repl.console.err(&format!("jev-code: cannot read keys from this terminal: {e}"));
            return 1;
        }
    };
    // The session moves into the task that runs a turn; the console is shared, so messages
    // typed meanwhile can still reach that turn.
    let console = repl.console.clone();
    let project = repl.engine.dir().to_path_buf();
    let state_dir = repl.state_dir().map(std::path::Path::to_path_buf);
    let mut workers_read = Instant::now();
    let tracked = lineedit::tracked_files(&project);
    let mut app = App::new(project.clone(), History::load(opts.history.as_deref()), repl.engine.mode(), route_label(&repl), tracked);
    app.fullscreen = FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst);
    refresh(&mut app, &repl);
    let (keymap, problems) = crate::workstation::keymap::load();
    app.keymap = keymap;
    if let Some(problem) = problems.first() {
        app.notice = Some((problem.clone(), Instant::now()));
    }

    let mut screen = Screen::new(std::io::stdout(), style.enabled);
    let mut events = Some(EventStream::new());
    let (stop_tx, stop_rx) = mpsc::unbounded_channel();
    let mut idle: Option<(Repl, Stops)> = Some((repl, Stops(stop_rx)));
    let mut task: Option<JoinHandle<Done>> = None;
    let mut mode_picked_during_turn = false;
    let mut tick = tokio::time::interval(term_env().tick(TICK));
    let mut last_width = width();
    // The answer the suggestion was last made from: a line that brings no new answer (a
    // command) makes none. A resumed session's last answer is not suggested from.
    let mut answer_seen: Option<String> = idle.as_ref().and_then(|(r, _)| r.engine.last_answer()).map(String::from);

    let code = loop {
        // Idle: catch up with the session, then run the next queued message, if any.
        if let Some((repl, _)) = idle.as_mut() {
            repl.engine.runtime.absorb_late();
            repl.console.flush_notices();
            // A provider that just became ready (or came off a cooldown) shows on the router row.
            app.providers = provider_chips(repl);
            if app.orchestrate {
                app.workers = worker_chips(state_dir.as_deref(), &project);
            }
            if app.mode != repl.engine.mode() {
                app.mode = repl.adopt_mode(app.mode);
            }
        }
        if task.is_none() && !app.suspended {
            if let Some(line) = app.queue.pop_front() {
                if let Some(started) = start(&mut idle, &line, style, &mut screen, &mut app) {
                    task = Some(started);
                }
            }
        }
        if !app.suspended {
            draw(&mut screen, &mut app, style);
        }

        let mut actions = tokio::select! {
            ev = next_event(&mut events) => match ev {
                Some(Ok(Event::Key(k))) => app.on_key(k, Instant::now()),
                Some(Ok(Event::Paste(text))) => {
                    let pasted = crate::termenv::sanitize_paste(&text);
                    app.on_paste(&pasted.text);
                    if pasted.truncated {
                        app.notice = Some(("the paste was too long: only the first 256 KiB was kept".to_string(), Instant::now()));
                    }
                    Vec::new()
                }
                // A resize to 0 columns (a window being dragged shut, some multiplexers
                // while detaching) says nothing about the real width.
                Some(Ok(Event::Resize(w, _))) if w == 0 => Vec::new(),
                Some(Ok(Event::Resize(w, _))) => {
                    let w = usize::from(w);
                    if app.fullscreen {
                        screen.forget();
                    } else if w < last_width {
                        screen.resized(w);
                    }
                    last_width = w;
                    Vec::new()
                }
                Some(Ok(Event::Mouse(m))) => {
                    match m.kind {
                        MouseEventKind::ScrollUp => app.scroll_chat(3),
                        MouseEventKind::ScrollDown => app.scroll_chat(-3),
                        _ => {}
                    }
                    Vec::new()
                }
                Some(Ok(_)) => Vec::new(),
                // The terminal went away: end the session.
                Some(Err(_)) | None => vec![Action::Quit],
            },
            msg = rx.recv() => match msg {
                Some(m) => app.on_front(m),
                None => Vec::new(),
            },
            _ = tick.tick(), if app.needs_ticks() => {
                app.expire(Instant::now());
                // Workers start and finish while a turn runs, in other processes: look at the
                // registry about once a second.
                if workers_read.elapsed() >= Duration::from_secs(1) {
                    workers_read = Instant::now();
                    app.workers = worker_chips(state_dir.as_deref(), &project);
                }
                Vec::new()
            }
            done = finished(&mut task) => {
                match done {
                    Some((repl, stops, flow)) => {
                        let flow_code = match flow {
                            Flow::Continue => None,
                            Flow::Quit => Some(0),
                            Flow::Aborted => Some(130),
                        };
                        if mode_picked_during_turn {
                            mode_picked_during_turn = false;
                        } else {
                            app.mode = repl.engine.mode();
                        }
                        refresh(&mut app, &repl);
                        let answer = repl.engine.last_answer();
                        if answer != answer_seen.as_deref() {
                            app.suggestion = answer.and_then(crate::gimmicks::suggest::next_prompt);
                            answer_seen = answer.map(String::from);
                        }
                        idle = Some((repl, stops));
                        if let Some(c) = flow_code {
                            break c;
                        }
                    }
                    // The line's task panicked: the session is lost with it.
                    None => break 1,
                }
                Vec::new()
            }
        };
        // Everything else the session said meanwhile, in order, before the next redraw.
        while let Ok(msg) = rx.try_recv() {
            actions.extend(app.on_front(msg));
        }
        let next = carry_out(actions, &mut app, &mut screen, &mut events, &stop_tx, &mut idle, &mut mode_picked_during_turn, style, &console);
        if let Some(started) = next.start {
            task = Some(started);
        }
        if let Some(c) = next.exit {
            break c;
        }
    };

    // Let the session print what it still has (a save error), then hand the terminal back.
    let mut late = Vec::new();
    while let Ok(FrontMsg::Print(text)) = rx.try_recv() {
        late.push(text);
    }
    if app.fullscreen {
        // The alternate screen is about to go: what is left to say goes on the real one.
        drop(events);
        keys_off();
        for text in late {
            println!("{text}");
        }
    } else {
        for text in late {
            screen.print_above(&text, &view::Frame { rows: Vec::new(), cursor: None });
        }
        screen.erase();
        drop(events);
        keys_off();
    }
    if let Some((mut repl, _)) = idle {
        if let Err(e) = repl.engine.save_now() {
            eprintln!("  could not save the session: {e}");
        }
    }
    code
}

/// What carrying out actions asks of the loop.
#[derive(Default)]
struct Next {
    start: Option<JoinHandle<Done>>,
    exit: Option<i32>,
}

/// Start running `line` on its own task, if the session is idle. The line is printed above
/// first, as it stays in the scrollback.
fn start(idle: &mut Option<(Repl, Stops)>, line: &str, style: Style, screen: &mut Screen<std::io::Stdout>, app: &mut App) -> Option<JoinHandle<Done>> {
    let (mut repl, mut stops) = idle.take()?;
    // Any line that runs answers the last answer, however it was sent (Enter, the queue,
    // Esc during a turn): its offer no longer stands, even when the line brings no answer.
    app.suggestion = None;
    let first = app.sent == 0;
    app.sent += 1;
    let block = crate::uifmt::sent_block(line, first, style, text_width(app));
    show(screen, app, &block, style);
    let line = line.to_string();
    Some(tokio::spawn(async move {
        let mut confirm = CardConfirm(repl.console.front());
        let flow = {
            let mut io = Io { interrupt: &mut stops, confirm: Some(&mut confirm) };
            repl.handle(commands::parse(&line), &mut io).await
        };
        (repl, stops, flow)
    }))
}

#[allow(clippy::too_many_arguments)]
fn carry_out(
    actions: Vec<Action>,
    app: &mut App,
    screen: &mut Screen<std::io::Stdout>,
    events: &mut Option<EventStream>,
    stop_tx: &mpsc::UnboundedSender<()>,
    idle: &mut Option<(Repl, Stops)>,
    mode_picked_during_turn: &mut bool,
    style: Style,
    console: &crate::console::Console,
) -> Next {
    let mut next = Next::default();
    for action in actions {
        match action {
            // The model may take it into the turn that is running; if it cannot, the message
            // waits in the queue for the next one.
            Action::Steer(text) => {
                console.steer(&text);
            }
            Action::Submit(line) => {
                if let Some(started) = start(idle, &line, style, screen, app) {
                    next.start = Some(started);
                } else {
                    // A line sent while the session is busy waits for it.
                    app.queue.push_back(line);
                }
            }
            Action::Stop => {
                let _ = stop_tx.send(());
            }
            Action::Quit => {
                next.exit = Some(0);
                return next;
            }
            Action::Print(text) => show(screen, app, &text, style),
            Action::Mode(_) => {
                if idle.is_none() {
                    *mode_picked_during_turn = true;
                }
            }
            Action::ClearScreen => {
                if app.fullscreen {
                    app.transcript.clear();
                } else {
                    screen.raw(crate::console::CLEAR_SCREEN);
                }
                screen.forget();
            }
            Action::Bell => screen.raw("\x07"),
            Action::Suspend(ack) => {
                screen.erase();
                // The key reader must let go of stdin before the command reads it.
                *events = None;
                keys_off();
                let _ = ack.send(());
            }
            Action::Resume => {
                if let Err(e) = keys_on() {
                    eprintln!("jev-code: cannot take the terminal back: {e}");
                    next.exit = Some(1);
                    return next;
                }
                screen.forget();
                *events = Some(EventStream::new());
            }
        }
    }
    next
}
